// Minimal model of the first T-0002 shim, QtCore only: a QObject that lives in thread R (like a QQuickImageResponse,
// which Qt's pixmap-reader thread creates) has its signal emitted from ANOTHER thread (the GUI thread, like
// response->deliver()). The signal is connected to a receiver in R whose slot calls sender()->deleteLater(), exactly what
// QQuickPixmapReader::asyncResponseFinished does. After posting the queued event, QMetaObject::activate still reads the
// sender's private data (cleanOrphanedConnections check) - if R already ran the slot AND the deferred delete in the
// meantime, that read hits freed memory.
//
// usage: race.exe <iterations> <hog-threads> [stall-us]
// stall-us > 0 adds a second, DIRECT slot that sleeps after the queued connection already posted its event. That only widens
// the window that a preempted emitting thread would have in the field (the race itself needs no stall).
#include "crash.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QThread>

#include <atomic>
#include <thread>
#include <vector>

static std::atomic_long g_dead{0};
class Victim : public QObject
{
    Q_OBJECT
public:
    ~Victim() override { g_dead.fetch_add(1); }
signals:
    void finished();
};

class Reaper : public QObject
{
    Q_OBJECT
public slots:
    void onFinished() { sender()->deleteLater(); }
};

int main(int argc, char** argv)
{
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    AddVectoredExceptionHandler(1, onException);
    QCoreApplication app(argc, argv);
    const long iterations = argc > 1 ? atol(argv[1]) : 100000;
    const int hogs = argc > 2 ? atoi(argv[2]) : 0;
    const int stallUs = argc > 3 ? atoi(argv[3]) : 0;

    std::atomic_bool stop{false};
    std::vector<std::thread> hog;
    for (int i = 0; i < hogs; ++i)
        hog.emplace_back([&] { volatile unsigned long x = 0; while (!stop.load(std::memory_order_relaxed)) ++x; });

    QThread r;
    Reaper reaper;
    reaper.moveToThread(&r);
    r.start();

    long deletedDuringEmit = 0;
    for (long i = 0; i < iterations; ++i) {
        Victim* v = nullptr;
        // create the victim IN thread R (blocking), as Qt's reader thread creates the response
        QMetaObject::invokeMethod(&reaper, [&] {
            v = new Victim;
            QObject::connect(v, &Victim::finished, &reaper, &Reaper::onFinished);   // AutoConnection -> queued across threads
            if (stallUs > 0)   // context = v itself would run in R; explicit DirectConnection so it runs in the emitting thread
                QObject::connect(v, &Victim::finished, &app, [stallUs] { QThread::usleep(stallUs); }, Qt::DirectConnection);
        }, Qt::BlockingQueuedConnection);
        const long before = g_dead.load();
        emit v->finished();   // emitted from the main thread: the bug
        if (g_dead.load() > before) ++deletedDuringEmit;
    }
    r.quit();
    r.wait();
    stop = true;
    for (auto& t : hog) t.join();
    printf("RACE DONE iterations=%ld stall_us=%d deleted_during_emit=%ld\n", iterations, stallUs, deletedDuringEmit);
    return 0;
}

#include "race.moc"
