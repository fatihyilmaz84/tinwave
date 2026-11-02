// Async image provider shim: QQuickAsyncImageProvider + bounded QThreadPool, pixels come from Rust.
//
// Threading: Qt docs say requestImageResponse() "may be called by multiple threads" (so no GUI-thread assumption),
// that finished() must be the response's final action even after cancel(), and that responses are deleted with
// deleteLater() once finished() was emitted. The worker therefore reports back through a queued signal of a task
// QObject connected to the response (Qt drops the call if the response is gone) instead of touching the response
// from another thread. (First version of this spike called response->deliver() from the GUI thread through a
// QPointer. Intermittent access violations in Qt6Core were seen while it was in use; the cause was never proven,
// see the ADR draft "Open risks".)
#include "image_provider.h"
#include "thumb_task.h"

#include <QtCore/QThreadPool>
#include <QtQuick/QQuickAsyncImageProvider>
#include <QtQuick/QQuickImageResponse>
#include <QtQuick/QQuickTextureFactory>

extern "C" int tw_render_thumbnail(const unsigned char* id, size_t id_len, int w, int h, unsigned char* out);

ThumbTask::ThumbTask(const QString& id, const QSize& size, std::shared_ptr<std::atomic_bool> cancelled)
    : m_id(id), m_size(size), m_cancelled(std::move(cancelled))
{
    setAutoDelete(true);
}

void ThumbTask::run()
{
    QImage img;
    if (!m_cancelled->load()) {
        img = QImage(m_size, QImage::Format_RGB32);
        const QByteArray utf8 = m_id.toUtf8();
        const int ok = tw_render_thumbnail(reinterpret_cast<const unsigned char*>(utf8.constData()),
                                           size_t(utf8.size()), m_size.width(), m_size.height(), img.bits());
        if (!ok) img = QImage();
    }
    emit done(img);
}

namespace {

class ThumbResponse : public QQuickImageResponse
{
public:
    ThumbResponse(const QString& id, const QSize& requested, QThreadPool* pool, int priority)
        : m_cancelled(std::make_shared<std::atomic_bool>(false))
    {
        // Size-specific thumbnail = whatever the Image asked for (logical sourceSize * devicePixelRatio).
        const QSize size = requested.isValid() ? requested : QSize(128, 128);
        auto* task = new ThumbTask(id, size, m_cancelled);
        connect(task, &ThumbTask::done, this, [this](const QImage& img) {
            m_image = img;
            emit finished();
        }, Qt::QueuedConnection);
        pool->start(task, priority);
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

class ThumbProvider : public QQuickAsyncImageProvider
{
public:
    ThumbProvider() { m_pool.setMaxThreadCount(2); } // bounded
    QQuickImageResponse* requestImageResponse(const QString& id, const QSize& requestedSize) override
    {
        // Newest request first: during a fast scroll the most recent rows are the visible ones.
        return new ThumbResponse(id, requestedSize, &m_pool, ++m_seq);
    }

private:
    QThreadPool m_pool;
    int m_seq = 0;
};

} // namespace

QQmlImageProviderBase* tw_make_image_provider() { return new ThumbProvider; }
