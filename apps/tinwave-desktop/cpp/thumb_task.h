#pragma once
// Worker task and shared state of the artwork image provider. Needs moc (Q_OBJECT + signal), so build.rs lists this
// header with cpp_file(): cxx-qt-build runs moc on it. The lifetime rules are in image_provider.cpp (ADR 0008,
// "image provider lifetime"); read them before changing anything here.
#include <QtCore/QObject>
#include <QtCore/QRunnable>
#include <QtCore/QSize>
#include <QtCore/QString>
#include <QtCore/QThreadPool>
#include <QtGui/QImage>

#include <atomic>
#include <cstdint>
#include <memory>
#include <mutex>

// State shared by the provider, all its responses and all its tasks. A shared_ptr keeps it alive for the last user, so
// a task or response that outlives the provider (they are deleted by Qt's reader thread) never sees freed memory.
struct ProviderShared
{
    ProviderShared(const void* rustCtx, int poolThreads, int maxEdge);
    // Releases the Rust context (see tw_provider_ctx_release); runs when the last task/response/handle is gone.
    ~ProviderShared();
    ProviderShared(const ProviderShared&) = delete;
    ProviderShared& operator=(const ProviderShared&) = delete;

    // Stops the provider: queued tasks skip their work and only emit done(); returns once no worker is running.
    // Setting `closing` and starting a task are serialised by `gate`, so after quiesce() returned no task can have
    // been, or ever be, started. GUI thread only (QThreadPool::waitForDone), idempotent.
    void quiesce();

    const void* rustCtx;                  // opaque Rust context; valid until ~ProviderShared
    const int maxEdge;                    // largest accepted thumbnail edge in pixels; the value is owned by Rust
    QThreadPool pool;                     // bounded (rule 4)
    std::mutex gate;                      // makes "closing is false, so start the task" atomic against quiesce()
    std::atomic_bool closing{false};      // set by quiesce() under `gate`; tasks then skip the work
    std::atomic_uint32_t seq{0};          // request counter, becomes the pool priority (newest first)

    // Observability for the teardown stress target and the "exactly one finished()" test. Relaxed atomics.
    std::atomic_uint64_t responses{0};    // responses created
    std::atomic_uint64_t finished{0};     // finished() emitted
    std::atomic_uint64_t duplicates{0};   // a second finished() was suppressed: must stay 0 (crash A in T-0041)
    std::atomic_uint64_t cancels{0};      // cancel() calls
    std::atomic_uint64_t tasksRun{0};     // tasks that asked Rust for pixels
    std::atomic_uint64_t tasksSkipped{0}; // tasks that skipped the work (cancelled or closing)
};

class ThumbTask : public QObject, public QRunnable
{
    Q_OBJECT
public:
    ThumbTask(const QString& id, const QSize& size, std::shared_ptr<std::atomic_bool> cancelled,
              std::shared_ptr<ProviderShared> shared);
    // Pool thread. Asks Rust for pixels, then emits done() as its LAST action. Never touches the response.
    // done(): image + empty error = served; empty image + error = Image.Error; both empty = cancelled or closing.
    void run() override;

signals:
    void done(const QImage& image, const QString& error);

private:
    QString m_id;
    QSize m_size;
    std::shared_ptr<std::atomic_bool> m_cancelled;
    std::shared_ptr<ProviderShared> m_shared;
};
