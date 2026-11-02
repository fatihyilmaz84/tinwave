// Provider variants for the T-0041 reproducer.
//  queued   : the T-0002 final pattern (task QObject + Qt::QueuedConnection into the response).
//  autoconn : the Qt documentation example pattern (same, default AutoConnection).
//  olda     : reconstruction of the first T-0002 shim: worker -> GUI thread (queued call on qApp) -> QPointer ->
//             response->deliver(); cancel() only sets a flag.
//  oldb     : like olda, but cancel() itself emits finished() (the response is then deleteLater'ed by Qt's reader
//             thread) and the late GUI-thread delivery still runs.
#include "providers.h"

#include <QtCore/QPointer>
#include <QtCore/QRandomGenerator>
#include <QtCore/QRunnable>
#include <QtCore/QThread>
#include <QtCore/QThreadPool>
#include <QtGui/QGuiApplication>
#include <QtGui/QImage>
#include <QtQuick/QQuickAsyncImageProvider>
#include <QtQuick/QQuickImageProvider>
#include <QtQuick/QQuickImageResponse>
#include <QtQuick/QQuickTextureFactory>

#include <atomic>
#include <mutex>
#include <memory>

using Flag = std::shared_ptr<std::atomic_bool>;

static QImage render(const QString& id, QSize size)
{
    // stand-in for file read + decode: 0..4 ms jitter and a deterministic fill
    QThread::usleep(QRandomGenerator::global()->bounded(4000));
    QImage img(size, QImage::Format_RGB32);
    img.fill(QColor::fromHsv(id.toUInt() * 47 % 360, 140, 200));
    return img;
}

namespace {

// ---------- queued / autoconn ----------
class ThumbTask : public QObject, public QRunnable
{
    Q_OBJECT
public:
    ThumbTask(const QString& id, const QSize& size, Flag c) : m_id(id), m_size(size), m_c(std::move(c)) { setAutoDelete(true); }
    void run() override
    {
        QImage img;
        if (!m_c->load()) img = render(m_id, m_size);
        emit done(img);
    }
signals:
    void done(const QImage& image);
private:
    QString m_id;
    QSize m_size;
    Flag m_c;
};

class QResponse : public QQuickImageResponse
{
public:
    QResponse(const QString& id, const QSize& req, QThreadPool* pool, int prio, Qt::ConnectionType ct)
        : m_c(std::make_shared<std::atomic_bool>(false))
    {
        const QSize size = req.isValid() ? req : QSize(128, 128);
        auto* task = new ThumbTask(id, size, m_c);
        connect(task, &ThumbTask::done, this, [this](const QImage& img) { m_image = img; emit finished(); }, ct);
        pool->start(task, prio);
    }
    QQuickTextureFactory* textureFactory() const override { return QQuickTextureFactory::textureFactoryForImage(m_image); }
    void cancel() override { m_c->store(true); }
private:
    QImage m_image;
    Flag m_c;
};

// ---------- olda / oldb ----------
class OldResponse;
class OldTask : public QRunnable
{
public:
    OldTask(const QString& id, const QSize& size, QPointer<OldResponse> r, Flag c) : m_id(id), m_size(size), m_r(std::move(r)), m_c(std::move(c)) { setAutoDelete(true); }
    void run() override;
private:
    QString m_id;
    QSize m_size;
    QPointer<OldResponse> m_r;
    Flag m_c;
};

class OldResponse : public QQuickImageResponse
{
public:
    OldResponse(const QString& id, const QSize& req, QThreadPool* pool, int prio, bool cancelEmits)
        : m_c(std::make_shared<std::atomic_bool>(false)), m_cancelEmits(cancelEmits)
    {
        const QSize size = req.isValid() ? req : QSize(128, 128);
        pool->start(new OldTask(id, size, QPointer<OldResponse>(this), m_c), prio);
    }
    void deliver(const QImage& img) { m_image = img; emit finished(); }   // called on the GUI thread (the bug under test)
    QQuickTextureFactory* textureFactory() const override { return QQuickTextureFactory::textureFactoryForImage(m_image); }
    void cancel() override
    {
        m_c->store(true);
        if (m_cancelEmits) emit finished();
    }
private:
    QImage m_image;
    Flag m_c;
    bool m_cancelEmits;
};

void OldTask::run()
{
    QImage img;
    if (!m_c->load()) img = render(m_id, m_size);
    QPointer<OldResponse> r = m_r;
    QMetaObject::invokeMethod(qGuiApp, [r, img] { if (r) r->deliver(img); }, Qt::QueuedConnection);
}

// ---------- guarded: plain QRunnable, a mutex-guarded target pointer that the response nulls in its destructor ----------
struct Guard
{
    std::mutex m;
    QObject* target = nullptr;   // the response while it is alive
};
class GResponse;
class GTask : public QRunnable
{
public:
    GTask(const QString& id, const QSize& size, std::shared_ptr<Guard> g, Flag c) : m_id(id), m_size(size), m_g(std::move(g)), m_c(std::move(c)) { setAutoDelete(true); }
    void run() override;
private:
    QString m_id;
    QSize m_size;
    std::shared_ptr<Guard> m_g;
    Flag m_c;
};
class GResponse : public QQuickImageResponse
{
public:
    GResponse(const QString& id, const QSize& req, QThreadPool* pool, int prio)
        : m_g(std::make_shared<Guard>()), m_c(std::make_shared<std::atomic_bool>(false))
    {
        m_g->target = this;
        pool->start(new GTask(id, req.isValid() ? req : QSize(128, 128), m_g, m_c), prio);
    }
    ~GResponse() override
    {
        std::lock_guard<std::mutex> l(m_g->m);
        m_g->target = nullptr;      // after this no worker will post to us; already-posted events die with the QObject
    }
    void finish(const QImage& img) { m_image = img; emit finished(); }
    QQuickTextureFactory* textureFactory() const override { return QQuickTextureFactory::textureFactoryForImage(m_image); }
    void cancel() override { m_c->store(true); }
private:
    QImage m_image;
    std::shared_ptr<Guard> m_g;
    Flag m_c;
};
void GTask::run()
{
    QImage img;
    if (!m_c->load()) img = render(m_id, m_size);
    std::lock_guard<std::mutex> l(m_g->m);
    if (m_g->target)
        QMetaObject::invokeMethod(m_g->target, [t = static_cast<GResponse*>(m_g->target), img] { t->finish(img); }, Qt::QueuedConnection);
}

// ---------- prod: Qt-doc pattern + explicit quiesce before engine teardown ----------
// Shared between the provider, every response and every task. Tasks only ever read `closing`.
struct Shared
{
    QThreadPool pool;
    std::atomic_bool closing{false};
    std::atomic_int seq{0};
};
std::weak_ptr<Shared>& registry() { static std::weak_ptr<Shared> w; return w; }

class ProdTask : public QObject, public QRunnable
{
    Q_OBJECT
public:
    ProdTask(const QString& id, const QSize& size, Flag cancelled, std::shared_ptr<Shared> shared)
        : m_id(id), m_size(size), m_cancelled(std::move(cancelled)), m_shared(std::move(shared)) { setAutoDelete(true); }
    void run() override   // pool thread. Never touches the response; the last action is the queued signal.
    {
        QImage img;
        if (!m_cancelled->load() && !m_shared->closing.load()) img = render(m_id, m_size);
        emit done(img);
    }
signals:
    void done(const QImage& image);
private:
    QString m_id;
    QSize m_size;
    Flag m_cancelled;
    std::shared_ptr<Shared> m_shared;
};

class ProdResponse : public QQuickImageResponse    // lives in (and is only touched from) Qt's pixmap-reader thread
{
public:
    ProdResponse(const QString& id, const QSize& req, std::shared_ptr<Shared> shared)
        : m_cancelled(std::make_shared<std::atomic_bool>(false))
    {
        if (shared->closing.load()) {   // provider is shutting down: complete empty, do not start work
            QMetaObject::invokeMethod(this, [this] { emit finished(); }, Qt::QueuedConnection);
            return;
        }
        auto* task = new ProdTask(id, req.isValid() ? req : QSize(128, 128), m_cancelled, shared);
        connect(task, &ProdTask::done, this, [this](const QImage& img) { m_image = img; emit finished(); }, Qt::QueuedConnection);
        shared->pool.start(task, ++shared->seq);
    }
    QQuickTextureFactory* textureFactory() const override { return QQuickTextureFactory::textureFactoryForImage(m_image); }
    void cancel() override { m_cancelled->store(true); }
private:
    QImage m_image;
    Flag m_cancelled;
};

class ProdProvider : public QQuickAsyncImageProvider
{
public:
    explicit ProdProvider(int threads) : m_shared(std::make_shared<Shared>())
    {
        m_shared->pool.setMaxThreadCount(threads);
        registry() = m_shared;
    }
    ~ProdProvider() override { quiesce(*m_shared); }
    QQuickImageResponse* requestImageResponse(const QString& id, const QSize& size) override
    {
        return new ProdResponse(id, size, m_shared);
    }
    static void quiesce(Shared& s)
    {
        s.closing.store(true);
        s.pool.waitForDone();   // queued tasks now skip the work and just emit; returns when no worker is running
    }
private:
    std::shared_ptr<Shared> m_shared;
};

// ---------- silent: pool tasks that run but never post anything back (responses never finish) ----------
// Separates "worker threads still running at teardown" from "worker threads posting events at teardown".
class SilentTask : public QRunnable
{
public:
    SilentTask(const QString& id, const QSize& size) : m_id(id), m_size(size) { setAutoDelete(true); }
    void run() override { render(m_id, m_size); }
private:
    QString m_id;
    QSize m_size;
};
class SilentResponse : public QQuickImageResponse
{
public:
    SilentResponse(const QString& id, const QSize& req, QThreadPool* pool, int prio)
    {
        pool->start(new SilentTask(id, req.isValid() ? req : QSize(128, 128)), prio);   // background work, never reports back
        QMetaObject::invokeMethod(this, [this] { emit finished(); }, Qt::QueuedConnection);   // completes by itself, on the reader thread
    }
    QQuickTextureFactory* textureFactory() const override { return nullptr; }
};

// ---------- selfq: no thread pool at all, the response completes itself with a queued call on the reader thread ----------
class SelfResponse : public QQuickImageResponse
{
public:
    SelfResponse(const QString& id, const QSize& req)
    {
        const QSize s = req.isValid() ? req : QSize(128, 128);
        m_image = render(id, s);
        QMetaObject::invokeMethod(this, [this] { emit finished(); }, Qt::QueuedConnection);
    }
    QQuickTextureFactory* textureFactory() const override { return QQuickTextureFactory::textureFactoryForImage(m_image); }
private:
    QImage m_image;
};

// ---------- sync: plain QQuickImageProvider (Qt runs requestImage() on its reader thread, no QObject hand-off) ----------
class SyncProvider : public QQuickImageProvider
{
public:
    SyncProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}
    QImage requestImage(const QString& id, QSize* size, const QSize& req) override
    {
        const QSize s = req.isValid() ? req : QSize(128, 128);
        if (size) *size = s;
        return render(id, s);
    }
};

class Provider : public QQuickAsyncImageProvider
{
public:
    Provider(QString variant, int threads) : m_variant(std::move(variant)) { m_pool.setMaxThreadCount(threads); }
    QQuickImageResponse* requestImageResponse(const QString& id, const QSize& size) override
    {
        const int prio = ++m_seq;
        if (m_variant == "queued") return new QResponse(id, size, &m_pool, prio, Qt::QueuedConnection);
        if (m_variant == "silent") return new SilentResponse(id, size, &m_pool, prio);
        if (m_variant == "selfq") return new SelfResponse(id, size);
        if (m_variant == "guarded") return new GResponse(id, size, &m_pool, prio);
        if (m_variant == "autoconn") return new QResponse(id, size, &m_pool, prio, Qt::AutoConnection);
        return new OldResponse(id, size, &m_pool, prio, m_variant == "oldb");
    }
private:
    QString m_variant;
    QThreadPool m_pool;
    std::atomic_int m_seq{0};
};

} // namespace

QQmlImageProviderBase* make_provider(const QString& variant, int threads)
{
    if (variant == "sync") return new SyncProvider;
    if (variant == "prod") return new ProdProvider(threads);
    return new Provider(variant, threads);
}

void quiesce_providers()
{
    if (auto s = registry().lock()) ProdProvider::quiesce(*s);
}

#include "providers.moc"
