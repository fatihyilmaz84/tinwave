// Async artwork image provider shim: QQuickAsyncImageProvider + bounded QThreadPool, pixels come from Rust.
//
// Lifetime rules (T-0041 root cause analysis; ADR 0008 "image provider lifetime"; docs/rules/qml-ui.md):
//  1. A QQuickImageResponse is created, signalled, cancelled and deleted (deleteLater) on Qt's pixmap-reader thread.
//     Worker threads never hold a pointer to it (not even a QPointer) and never call into it. No pointer to a
//     response leaves the reader thread: the only things that cross are the task's queued done() signal and the
//     atomic cancel flag (a shared_ptr<atomic_bool>, not a pointer to the response).
//  2. The only worker -> response channel is a queued signal of the task (sender) connected with the response as the
//     context object. Qt's connection machinery drops the call if the response is gone. emit done() is the task's
//     last action.
//  3. Exactly one finished() per response. cancel() only sets the atomic flag; the cancelled task still emits done()
//     with an empty image, which is the single place that completes the response (plus the closing path, which does
//     not start a task). complete() refuses a second emission and counts it: emitting twice crashed in
//     QObject::deleteLater (crash A, 120/120 runs).
//  4. A bounded QThreadPool in shared state held by provider, responses and tasks, newest-request-first, with a
//     closing flag checked before any work. "closing is false, so start the task" (response constructor) and "set
//     closing" (quiesce) both run under ProviderShared::gate, so once quiesce() has set the flag no task can be
//     started any more and waitForDone() then really waits for the last one.
//  5. Teardown: tasks in flight while Qt destroys the QML engine (and with it the pixmap-reader QThread that owns the
//     responses) crashed inside ~QThread (crash B, 4/400 real-app runs). The owner calls tw_provider_quiesce() after
//     the event loop ended and BEFORE the engine is destroyed: it sets `closing` and waits until no worker is running.
//     After that nothing outside Qt's own threads touches responses, and requests arriving late complete empty
//     without starting a task. The provider destructor repeats the quiesce; it is not relied upon (2/300 hangs).
//
// What QML sees (T-0049): a served thumbnail is Image.Ready; a refused size and a key without artwork are
// Image.Error (the response reports errorString(), so the Image can show its placeholder by status); a cancelled or
// shutdown-time response stays silent (empty image, no error string: QML has already moved on).
#include "image_provider.h"
#include "thumb_task.h"

#include <QtCore/QMetaObject>
#include <QtQuick/QQuickAsyncImageProvider>
#include <QtQuick/QQuickImageResponse>
#include <QtQuick/QQuickTextureFactory>

#include <memory>
#include <utility>

// Implemented in Rust (src/app/artwork_bridge.rs).
extern "C" int32_t tw_render_thumbnail(const void* ctx, const unsigned char* id, size_t idLen, int32_t w, int32_t h,
                                       unsigned char* out);
extern "C" void tw_provider_ctx_release(const void* ctx);

namespace {
// Edge of the square thumbnail when the Image gave no usable sourceSize (QML should always set it: logical pixels, Qt
// multiplies by the device pixel ratio itself).
constexpr int kDefaultEdge = 128;

// The size a request asks for, resolved from the sourceSize the Image gave (logical size times device pixel ratio, in
// pixels; Qt reports 0 or -1 for a dimension QML left unset):
//  * both dimensions set: used as is;
//  * one dimension set: that edge is kept and the other one equals it. Qt itself would derive the other edge from the
//    image's aspect ratio, but the provider cannot know the aspect here: this runs on the reader thread, which must
//    not wait for Rust, the DB or a decode, and the source returns pixels at exactly the size asked for. A square is
//    right for album covers; a source serving other aspects needs an "aspect of key" query first (not built, nothing
//    needs it yet);
//  * neither set: the default square.
QSize resolveSize(const QSize& requested)
{
    const int w = requested.width();
    const int h = requested.height();
    if (w > 0 && h > 0) return requested;
    if (w > 0) return QSize(w, w);
    if (h > 0) return QSize(h, h);
    return QSize(kDefaultEdge, kDefaultEdge);
}
} // namespace

ProviderShared::ProviderShared(const void* ctx, int poolThreads, int maxEdgePx) : rustCtx(ctx), maxEdge(maxEdgePx)
{
    pool.setMaxThreadCount(poolThreads < 1 ? 1 : poolThreads);
}

ProviderShared::~ProviderShared()
{
    // Runs when the last holder is gone. Tasks hold a reference, so none can still be using the Rust context here;
    // normally the last holder is the Rust handle, dropped on the GUI thread after the quiesce and the engine.
    tw_provider_ctx_release(rustCtx);
}

void ProviderShared::quiesce()
{
    {
        std::lock_guard<std::mutex> lock(gate);
        closing.store(true);
    }
    pool.waitForDone(); // queued tasks now skip the work and only emit; returns when no worker is running
}

ProviderHandle::ProviderHandle(std::shared_ptr<ProviderShared> s) : shared(std::move(s)) {}
ProviderHandle::~ProviderHandle() = default;

ThumbTask::ThumbTask(const QString& id, const QSize& size, std::shared_ptr<std::atomic_bool> cancelled,
                     std::shared_ptr<ProviderShared> shared)
    : m_id(id), m_size(size), m_cancelled(std::move(cancelled)), m_shared(std::move(shared))
{
    setAutoDelete(true);
}

void ThumbTask::run()
{
    QImage img;
    QString error; // empty with an empty image = cancelled or shutting down: the response completes silently
    if (!m_cancelled->load() && !m_shared->closing.load()) {
        m_shared->tasksRun.fetch_add(1, std::memory_order_relaxed);
        img = QImage(m_size, QImage::Format_RGB32); // m_size is within 1..maxEdge (the response checked)
        if (!img.isNull()) { // allocation can fail; never hand Rust a null buffer
            const QByteArray utf8 = m_id.toUtf8();
            const int32_t ok = tw_render_thumbnail(m_shared->rustCtx, reinterpret_cast<const unsigned char*>(utf8.constData()),
                                                   size_t(utf8.size()), m_size.width(), m_size.height(), img.bits());
            if (!ok) {
                img = QImage();
                error = QStringLiteral("no artwork for \"%1\" at %2x%3").arg(m_id).arg(m_size.width()).arg(m_size.height());
            }
        } else {
            error = QStringLiteral("out of memory for a %1x%2 thumbnail").arg(m_size.width()).arg(m_size.height());
        }
    } else {
        m_shared->tasksSkipped.fetch_add(1, std::memory_order_relaxed);
    }
    emit done(img, error); // last statement: after this the response may run (and be deleted) on the reader thread
}

namespace {

class ThumbResponse : public QQuickImageResponse
{
public:
    ThumbResponse(const QString& id, const QSize& requested, std::shared_ptr<ProviderShared> shared)
        : m_cancelled(std::make_shared<std::atomic_bool>(false)), m_shared(std::move(shared))
    {
        m_shared->responses.fetch_add(1, std::memory_order_relaxed);
        // Size-specific thumbnail = whatever the Image asked for (logical sourceSize * devicePixelRatio).
        const QSize size = resolveSize(requested);
        const int maxEdge = m_shared->maxEdge; // the limit comes from Rust (ProviderCore), see ProviderShared
        if (size.width() > maxEdge || size.height() > maxEdge) { // refuse: Image.Error, allocate nothing, run nothing
            m_error = QStringLiteral("thumbnail size %1x%2 is above the limit of %3 px per edge")
                          .arg(size.width()).arg(size.height()).arg(maxEdge);
            QMetaObject::invokeMethod(this, [this] { complete(); }, Qt::QueuedConnection);
            return;
        }
        auto task = std::make_unique<ThumbTask>(id, size, m_cancelled, m_shared);
        connect(task.get(), &ThumbTask::done, this, [this](const QImage& img, const QString& error) {
            m_image = img;
            m_error = error;
            complete();
        }, Qt::QueuedConnection);
        {
            // Atomic against quiesce(): either closing is already set and we start nothing, or the task is in the
            // pool before quiesce() can set it, and its waitForDone() waits for it.
            std::lock_guard<std::mutex> lock(m_shared->gate);
            if (!m_shared->closing.load()) {
                // Newest request first: during a fast scroll the most recent rows are the visible ones. The counter
                // wraps after 2^31 requests; the only effect is one short stretch of mis-ordered priorities.
                m_shared->pool.start(task.release(),
                                     int(m_shared->seq.fetch_add(1, std::memory_order_relaxed) & 0x7fffffffu));
                return;
            }
        }
        // Shutting down: complete empty, start no work (the unstarted task is deleted by the unique_ptr).
        QMetaObject::invokeMethod(this, [this] { complete(); }, Qt::QueuedConnection);
    }

    QQuickTextureFactory* textureFactory() const override
    {
        return QQuickTextureFactory::textureFactoryForImage(m_image);
    }

    QString errorString() const override { return m_error; }

    void cancel() override
    {
        m_shared->cancels.fetch_add(1, std::memory_order_relaxed);
        m_cancelled->store(true); // flag only: finished() still comes from the task's done() (rule 3)
    }

private:
    // The only place finished() is emitted. Reader thread only.
    void complete()
    {
        if (m_completed) {
            m_shared->duplicates.fetch_add(1, std::memory_order_relaxed);
            return;
        }
        m_completed = true;
        m_shared->finished.fetch_add(1, std::memory_order_relaxed);
        emit finished();
    }

    QImage m_image;
    QString m_error;
    bool m_completed = false;
    std::shared_ptr<std::atomic_bool> m_cancelled;
    std::shared_ptr<ProviderShared> m_shared;
};

class ThumbProvider : public QQuickAsyncImageProvider
{
public:
    explicit ThumbProvider(std::shared_ptr<ProviderShared> shared) : m_shared(std::move(shared)) {}
    ~ThumbProvider() override { m_shared->quiesce(); }
    QQuickImageResponse* requestImageResponse(const QString& id, const QSize& requestedSize) override
    {
        return new ThumbResponse(id, requestedSize, m_shared);
    }

private:
    std::shared_ptr<ProviderShared> m_shared;
};

} // namespace

std::unique_ptr<ProviderHandle> tw_provider_handle_new(const uint8_t* rustCtx, int32_t poolThreads, int32_t maxEdge)
{
    return std::make_unique<ProviderHandle>(std::make_shared<ProviderShared>(rustCtx, int(poolThreads), int(maxEdge)));
}

QQmlImageProviderBase* tw_provider_make(const ProviderHandle& handle) { return new ThumbProvider(handle.shared); }

void tw_provider_quiesce(const ProviderHandle& handle) { handle.shared->quiesce(); }

uint64_t tw_provider_counter(const ProviderHandle& handle, uint32_t which)
{
    const ProviderShared& s = *handle.shared;
    switch (which) {
    case 0: return s.responses.load(std::memory_order_relaxed);
    case 1: return s.finished.load(std::memory_order_relaxed);
    case 2: return s.duplicates.load(std::memory_order_relaxed);
    case 3: return s.cancels.load(std::memory_order_relaxed);
    case 4: return s.tasksRun.load(std::memory_order_relaxed);
    case 5: return s.tasksSkipped.load(std::memory_order_relaxed);
    default: return 0;
    }
}
