#pragma once
// Worker task of the async image provider. Needs moc (Q_OBJECT + signal), so build.rs lists this header
// with cpp_file(): cxx-qt-build runs moc on it. See image_provider.cpp for the lifetime rules (T-0041).
#include <QtCore/QObject>
#include <QtCore/QRunnable>
#include <QtCore/QSize>
#include <QtCore/QString>
#include <QtCore/QThreadPool>
#include <QtGui/QImage>

#include <atomic>
#include <memory>

// State shared by the provider, all its responses and all its tasks (shared_ptr keeps it alive for the last user).
struct ProviderShared
{
    QThreadPool pool;
    std::atomic_bool closing{false}; // set by tw_image_provider_shutdown(); tasks then skip the work
    std::atomic_int seq{0};
};

class ThumbTask : public QObject, public QRunnable
{
    Q_OBJECT
public:
    ThumbTask(const QString& id, const QSize& size, std::shared_ptr<std::atomic_bool> cancelled,
              std::shared_ptr<ProviderShared> shared);
    // Pool thread. Asks Rust for pixels, then emits done() as its LAST action. Never touches the response.
    void run() override;

signals:
    void done(const QImage& image);

private:
    QString m_id;
    QSize m_size;
    std::shared_ptr<std::atomic_bool> m_cancelled;
    std::shared_ptr<ProviderShared> m_shared;
};
