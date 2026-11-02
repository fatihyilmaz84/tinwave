// Async image provider shim: QQuickAsyncImageProvider + bounded QThreadPool, pixels come from Rust.
//
// Lifetime rules (T-0041 root cause analysis; see crash-T-0041/ and ADR 0008):
//  1. A QQuickImageResponse is created, signalled, cancelled and deleted (deleteLater) on Qt's pixmap-reader thread.
//     Worker threads must never hold a pointer to it and never call into it.
//  2. The only worker -> response channel is a queued signal of the task (sender) connected with the response as
//     context object. Qt's connection machinery drops the call if the response is gone, so no QPointer/raw pointer
//     is ever dereferenced across threads. emit done() is the task's last action.
//  3. cancel() only sets an atomic flag. The task still emits done() (empty image), so finished() is always emitted
//     exactly once and Qt can delete the response.
//  4. Teardown: tasks in flight while Qt destroys the QML engine (and with it the pixmap-reader QThread that owns
//     the responses) crashed inside ~QThread in about 1-3 percent of stress runs. The owner therefore calls
//     tw_image_provider_shutdown() after the event loop ends and BEFORE the engine is destroyed: it sets `closing`
//     (queued tasks skip the work) and waits until no worker thread is running. After that nothing outside Qt's own
//     threads touches responses, and requests arriving late complete empty without starting a task.
#include "image_provider.h"
#include "thumb_task.h"

#include <QtCore/QThreadPool>
#include <QtQuick/QQuickAsyncImageProvider>
#include <QtQuick/QQuickImageResponse>
#include <QtQuick/QQuickTextureFactory>

#include <memory>

extern "C" int tw_render_thumbnail(const unsigned char* id, size_t id_len, int w, int h, unsigned char* out);

ThumbTask::ThumbTask(const QString& id, const QSize& size, std::shared_ptr<std::atomic_bool> cancelled,
                     std::shared_ptr<ProviderShared> shared)
    : m_id(id), m_size(size), m_cancelled(std::move(cancelled)), m_shared(std::move(shared))
{
    setAutoDelete(true);
}

void ThumbTask::run()
{
    QImage img;
    if (!m_cancelled->load() && !m_shared->closing.load()) {
        img = QImage(m_size, QImage::Format_RGB32);
        const QByteArray utf8 = m_id.toUtf8();
        const int ok = tw_render_thumbnail(reinterpret_cast<const unsigned char*>(utf8.constData()),
                                           size_t(utf8.size()), m_size.width(), m_size.height(), img.bits());
        if (!ok) img = QImage();
    }
    emit done(img);
}

namespace {

std::weak_ptr<ProviderShared>& registry()
{
    static std::weak_ptr<ProviderShared> w; // only touched on the GUI thread (provider ctor, shutdown)
    return w;
}

class ThumbResponse : public QQuickImageResponse
{
public:
    ThumbResponse(const QString& id, const QSize& requested, const std::shared_ptr<ProviderShared>& shared)
        : m_cancelled(std::make_shared<std::atomic_bool>(false))
    {
        if (shared->closing.load()) { // shutting down: complete empty, start no work
            QMetaObject::invokeMethod(this, [this] { emit finished(); }, Qt::QueuedConnection);
            return;
        }
        // Size-specific thumbnail = whatever the Image asked for (logical sourceSize * devicePixelRatio).
        const QSize size = requested.isValid() ? requested : QSize(128, 128);
        auto* task = new ThumbTask(id, size, m_cancelled, shared);
        connect(task, &ThumbTask::done, this, [this](const QImage& img) {
            m_image = img;
            emit finished();
        }, Qt::QueuedConnection);
        // Newest request first: during a fast scroll the most recent rows are the visible ones.
        shared->pool.start(task, ++shared->seq);
    }

    QQuickTextureFactory* textureFactory() const override
    {
        return QQuickTextureFactory::textureFactoryForImage(m_image);
    }
    void cancel() override { m_cancelled->store(true); }

private:
    QImage m_image;
    std::shared_ptr<std::atomic_bool> m_cancelled;
};

void quiesce(ProviderShared& s)
{
    s.closing.store(true);
    s.pool.waitForDone(); // queued tasks now skip the work and only emit; returns when no worker is running
}

class ThumbProvider : public QQuickAsyncImageProvider
{
public:
    ThumbProvider() : m_shared(std::make_shared<ProviderShared>())
    {
        m_shared->pool.setMaxThreadCount(2); // bounded
        registry() = m_shared;
    }
    ~ThumbProvider() override { quiesce(*m_shared); }
    QQuickImageResponse* requestImageResponse(const QString& id, const QSize& requestedSize) override
    {
        return new ThumbResponse(id, requestedSize, m_shared);
    }

private:
    std::shared_ptr<ProviderShared> m_shared;
};

} // namespace

QQmlImageProviderBase* tw_make_image_provider() { return new ThumbProvider; }

void tw_image_provider_shutdown()
{
    if (auto s = registry().lock()) quiesce(*s);
}
