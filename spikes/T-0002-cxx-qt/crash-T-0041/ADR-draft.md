# ADR 0008 addendum (draft) — Async image provider: lifetime rules and teardown (T-0041)

- **Status:** Proposed (draft from spike T-0041; the lead folds this into ADR 0008, decision 5 and "Open risks")
- **Date:** 2026-11-02

## Context

T-0002 saw 5 access violations in `Qt6Core.dll` (RVA 0xe9f73, QObject/signal machinery) in early runs of the deployed
release build and none after the async image provider was switched to a queued-signal hand-off. ADR 0008 decision 5
forbids reusing the provider until the cause is explained. No stack was ever captured and the first shim is not in git
history (the spike is a single commit), so this spike rebuilt the provider as a C++-only harness
(`spikes/T-0002-cxx-qt/crash-T-0041/`) with a stress QML (60 image items flipping source every 1-5 ms, a `GridView`
jumped to random positions every 4 ms with `cacheBuffer: 0`, 30 delegates created and destroyed every 3 ms, random
`Qt.quit()` between 50 and 900 ms so the engine is torn down with requests in flight), a vectored exception handler
that writes a stack plus a minidump, and Debug (Qt debug DLLs), Debug + MSVC AddressSanitizer and Release builds. (ASan only instruments our code; the Qt DLLs are not instrumented, so
it found nothing and is weak evidence; the Debug CRT heap and the Release build did the work.)
Machine: Windows 11 Pro 10.0.26200, Ryzen 7 7800X3D (16 threads), Qt 6.12.0 msvc2022_64, MSVC 14.50.35717. Qt ships no
PDBs in this install, so frames inside Qt show only the RVA (the symbolizer prints the nearest export, which is misleading).

## Findings

1. **How Qt uses a response (Qt source, qquickpixmapcache.cpp, Qt 6.10 branch, read as documentation):** the pixmap-reader
   thread calls `requestImageResponse()`, so every `QQuickImageResponse` is a QObject owned by the *reader thread*, not the
   GUI thread. `finished` is connected (AutoConnection) to the reader's enforcer, whose slot calls `response->deleteLater()`
   using `sender()`. `cancel()` is also called on the reader thread.
2. **Reproduced crash A, double `finished()` (stack captured).** If a response emits `finished()` twice (reconstructed
   variant "oldb": `cancel()` emits `finished()` and the late GUI-thread delivery emits it again) the second queued
   delivery finds `sender() == nullptr` because the first one already deleted the response, and the reader thread crashes in
   `QObject::deleteLater` (read at address 0x8) called from `QQuickPixmapReader::asyncResponseFinished`
   (Release: 120 of 120 runs, Qt6Core RVA 0xf0780; Debug: 69 of 160). The variant "olda" (GUI-thread delivery through
   `QPointer`, `cancel()` only sets a flag, one `finished()` per response) did **not** crash in 40 Debug + 420 Release runs,
   and a QtCore-only model of "emit from a foreign thread, the owner thread deletes the sender meanwhile"
   (`race.cpp`, 2000 of 2000 senders deleted during the emission, widened with a 300 us stall) also survived in Release and
   Debug. So a single cross-thread `emit` is not by itself fatal in Qt 6.12; the first shim crashed only if something
   more was wrong (double `finished()`, or use of the response after deletion).
3. **Reproduced crash B, teardown with worker tasks in flight (this applies to the T-0002 "final" pattern too).** The
   queued-signal pattern is correct while the app runs but crashes in about **1-3 percent of runs when the QML engine is
   destroyed while pool tasks are still queued or running**: access violation inside `QThread` teardown
   (`QObject::~QObject` -> `QObjectPrivate::deleteChildren` -> Qt Quick reader `QThread` destructor -> Qt6Core RVA 0x199897,
   Release; heap corruption / `0xC0000374` in Debug), on the GUI thread, after `exec()` returned. Counts, same stress
   settings (Release, 1 pool thread, exit 200-900 ms): `queued` 3 of 300, `guarded` (mutex-guarded target pointer, no
   task QObject) 8 of 300, `autoconn` (Qt doc example) 4 of 40 in Debug. Controls that never crash: synchronous provider
   (`sync`) 0 of 310, no pool at all (`selfq`) 0 of 250, no images (`noimg`) 0 of 60, and **the queued pattern when
   the storm is stopped 400 ms before quitting 0 of 250**. So the necessary condition is pool tasks alive (and posting events
   to reader-thread objects) while the engine is destroyed; the response hand-off design is not what matters. The precise
   Qt-internal frame is not symbolized (no Qt PDBs), the diagnosis is by elimination and ablation, not by a Qt-side stack.
   The real Rust-backed spike app shows the same thing: with the T-0002 final provider and no shutdown call, 4 of 400
   stress runs crashed (two at Qt6Core RVA 0x199897 on the main thread with exactly the harness frames 0x1994d7, 0xebfa6,
   Qt6Quick 0x20ba64, 0xf0650, 0xebf68; two on thread 2 in `VCRUNTIME140` called from Qt6Core RVA 0x198645/0x1996f8, the same
   QThread-teardown area; exit codes 0xC0000005 and 0xC0000374). **Whether this is also the original 0xe9f73 crash is
   unproven**: that RVA never appeared. The disassembly at 0xe9f73 (`mov rax,[rcx+20h]` right after the `QObjectPrivate::ConnectionData`
   refcount release in the tail of the signal-emission code, with `rcx = [obj+0x60]`) says the original fault was a read
   through a freed QObject private/connection data during or right after a signal emission. The early crashes happened in
   benchmark runs that end with `Qt.quit()`, i.e. in the same teardown window, and the first shim started
   worker-to-GUI hand-offs that are still pending at that time, so teardown is the most likely class, but it is not proven.
4. **Fix, verified.** Tell the provider to quiesce after the event loop ends and **before** the engine is destroyed
   (`tw_image_provider_shutdown()`): set a `closing` flag (queued tasks skip the work and only emit, new requests complete
   empty without starting a task) and `QThreadPool::waitForDone()`. Acceptance, harness, 0 failures in:
   Debug 300 + 150 (toggle mode), ASan 150, Release 500 + 300 + 200 (scroll-only, 8 in parallel), Release with a real window
   (software scene graph, `windows` platform) 40 = **1640 runs**. Real Rust-backed spike app (`T2_MODE=stress`, Release,
   FluentWinUI3, offscreen): fixed 400 of 400 clean; baseline (T-0002 final provider, no shutdown call) 4 of 400 crashed
   (`results/summary-real-baseline.txt`, `results/summary-real.txt`). Total clean runs of the fixed pattern: 2040.
5. A side effect that matters for production: with an unbounded queue and responses that complete immediately, one pool
   thread builds a backlog of thousands of tasks during a scroll storm and `~QThreadPool` waits for all of them at exit
   (a control variant without any cancel/closing check made a batch of 400 runs take more than 10 minutes and was
   abandoned). The `closing`/`cancelled` flags make queued work cost nothing, so the exit is immediate.

## Decision (recommended production pattern)

1. **Ownership.** A response is created, signalled, cancelled and deleted on Qt's reader thread. Worker threads never
   hold a pointer to a response and never call into it, not even through `QPointer` (checking a `QPointer` and then
   using the pointer is a race against `deleteLater()` on another thread).
2. **Hand-off.** Worker task = `QObject + QRunnable` (autoDelete). The only worker-to-response channel is the task's
   queued signal `done(QImage)`, connected with the response as the context object (`Qt::QueuedConnection`, lambda or
   slot). Qt drops the call if the response is gone. `emit done()` is the last statement of `run()`.
3. **Exactly one `finished()`.** The slot that stores the image and emits `finished()` runs once. `cancel()` only sets an
   `std::atomic_bool` shared with the task; a cancelled task still emits `done()` with an empty image, because Qt deletes
   a response only after `finished()`. Never emit `finished()` from `cancel()`.
4. **Bounded and cheap to drop.** A bounded `QThreadPool` owned by shared state (`std::shared_ptr`) that the provider,
   responses and tasks all hold; newest request first (priority = increasing sequence).
5. **Teardown.** The application calls `tw_image_provider_shutdown()` (closing flag + `waitForDone()`) on the GUI thread
   right after `QGuiApplication::exec()` returns and **before** the `QQmlApplicationEngine` is dropped (in Rust:
   `app.exec(); image::tw_image_provider_shutdown(); drop(engine);`). The provider destructor repeats it (idempotent).
   The Rust render function called by the task must be bounded in time and must not take locks that the GUI thread holds.
6. **Tests that stay.** A CI/dev stress target equivalent to `crash-T-0041/run-batch.ps1` (random exit 200-900 ms, 1 pool
   thread, 300 runs) must stay green; any change to `cpp/image_provider.cpp` reruns it.

## Lifetime argument for the pattern in 1-5

- *Response alive while a task posts:* the task posts through `QMetaObject::activate` on a connection whose receiver is the
  response. `activate` and `~QObject` (which removes the connection and the posted events of the receiver) both take the
  connection mutex, so either the event is posted before destruction (and removed by `removePostedEvents` in the
  destructor) or the connection is already gone. The task (sender) is alive for the whole `run()` because the pool owns
  it. No raw or guarded pointer to the response exists outside the reader thread.
- *Single completion:* one connection fires once per task; `cancel()` does not emit; so `asyncResponseFinished` runs once
  per response and `sender()` is valid.
- *Image data:* `m_image` is written by the slot on the reader thread before `finished()` and read by `textureFactory()`
  on the same thread. The pixel buffer comes from the worker as a `QImage` (implicitly shared, detached before the
  signal), not a pointer into worker memory.
- *Teardown:* after `tw_image_provider_shutdown()` returns, no worker thread exists that could post to reader-thread
  objects, the pool is idle (and refuses to start new work), so the engine destructor and the reader `QThread`
  destructor run with no concurrent producers. Late requests arriving during teardown complete on the reader thread
  itself (queued self-call) without touching the pool.

## Consequences

- One more call in `main` and a small shared state struct; the provider cannot be used without the shutdown hook
  (documented in `image_provider.h`). Production apps with several providers need a registry; this spike has one.
- Qt PDBs were not available, so crash B is explained by elimination, not by a Qt-side stack. If a future Qt version
  ships PDBs or the crash reappears in the field, re-run the harness first.
- No new dependency; MSVC ASan and the Qt debug DLLs that ship in the kit were used. Nothing was installed system-wide.

## Spec / docs updated

None by the spike. The lead should update ADR 0008 decision 5 and "Open risks", `docs/rules/qml-ui.md` (image provider
rules 1-5) and add the stress target to `docs/rules/testing.md`.
