#pragma once
// Worker task of the async image provider. Needs moc (Q_OBJECT + signal), so build.rs lists this header
// with cpp_file(): cxx-qt-build runs moc on it. See image_provider.cpp for why this is a QObject.
#include <QtCore/QObject>
#include <QtCore/QRunnable>
#include <QtCore/QSize>
#include <QtCore/QString>
#include <QtGui/QImage>

#include <atomic>
#include <memory>

class ThumbTask : public QObject, public QRunnable
{
    Q_OBJECT
public:
    ThumbTask(const QString& id, const QSize& size, std::shared_ptr<std::atomic_bool> cancelled);
    void run() override; // pool thread: asks Rust for pixels, then emits done()

signals:
    void done(const QImage& image);

private:
    QString m_id;
    QSize m_size;
    std::shared_ptr<std::atomic_bool> m_cancelled;
};
