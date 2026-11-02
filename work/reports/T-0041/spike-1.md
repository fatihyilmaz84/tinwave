SPIKE RESULT T-0041
answer: The original RVA 0xe9f73 crash is not reproduced, so it stays unproven. What I did reproduce and fix is a separate, real crash in the queued-signal pattern (about 1-3% of runs): the QML engine is destroyed while pool tasks are still in flight. Recommended production pattern is the existing queued-signal hand-off plus an explicit provider shutdown (closing flag + QThreadPool::waitForDone()) between the end of exec() and the engine's destruction.
confidence: medium (high that the queued pattern has a teardown crash and that the shutdown fix removes it; low that this is the same crash as the original 0xe9f73)
evidence:
- Machine and builds: Windows 11 Pro 10.0.26200, Ryzen 7 7800X3D (16 threads), Qt 6.12.0 msvc2022_64, MSVC 14.50.35717. The first shim is not in git history (the spike is one commit), so I built a C++-only harness: spikes/T-0002-cxx-qt/crash-T-0041/. It has several provider variants, a stress QML, and a vectored exception handler that writes a stack and a minidump. Builds: Debug with Qt's debug DLLs, Debug + MSVC ASan, and Release.
- Stress used: 60 images flipping source every 1-5 ms, a GridView jumped to random positions every 4 ms with cacheBuffer 0, 30 delegates churned every 3 ms, and Qt.quit() at a random 50-900 ms. Qt ships no PDBs here, so frames inside Qt show only an RVA (the nearest-export names are misleading). dumpbin disassembly was used on the release DLL.
- How Qt uses a response (qquickpixmapcache.cpp, Qt 6.10 branch): the reader thread calls requestImageResponse(), so every response is a QObject owned by the pixmap-reader thread, not the GUI thread. finished() goes to the reader's enforcer, which calls response->deleteLater() using sender(). cancel() is also called on the reader thread.
- Crash A, reproduced, stack captured: a response that emits finished() twice (variant oldb: cancel() emits it, and a late GUI-thread delivery emits it again). Release crashed in 120/120 runs and Debug in 69/160. Frame: QObject::deleteLater (Qt6Core RVA 0xf0780, read at 0x8) called from QQuickPixmapReader::asyncResponseFinished (Qt6Quick RVA 0x20cc2e). The cause is sender()==nullptr after the first delivery deleted the response.
- Variant olda (GUI-thread delivery through QPointer, single finished()) did not crash in 40 Debug + 420 Release runs. A QtCore-only model of "foreign thread emits on a QObject that its owner thread deletes meanwhile" (race.cpp) also survived. All 2000 senders were deleted during the emission, widened with a 300 us stall, in Release and Debug. So a single cross-thread emit is not by itself fatal in Qt 6.12.
- Crash B, reproduced in the real Rust-backed spike app, which I added a T2_MODE=stress to: with the T-0002 final provider and no shutdown call, 4 of 400 stress runs crashed (Release, FluentWinUI3, offscreen). In the harness, queued crashed 3/300 and guarded (mutex-guarded pointer, no task QObject) 8/300 in Release with 1 pool thread and exit at 200-900 ms; autoconn (the Qt doc example) crashed 4/40 in Debug.
- Crash B stacks, from the real app and the harness: main thread, after exec() returned, ~QObject -> QObjectPrivate::deleteChildren -> Qt Quick reader-QThread destructor -> Qt6Core RVA 0x199897 (frames 0x1994d7, 0xebfa6, Qt6Quick 0x20ba64, 0xf0650, 0xebf68). Two of the four real-app crashes were instead on thread 2, in VCRUNTIME140 called from Qt6Core RVA 0x198645/0x1996f8, the same QThread-teardown area. Exit codes 0xC0000005 and 0xC0000374.
- Controls that never crashed: synchronous provider 0/310; no thread pool at all 0/250; no images 0/60; queued pattern with the storm stopped 400 ms before quit 0/250. So the trigger is pool tasks alive and posting to reader-thread objects while the engine is destroyed. Crash B is diagnosed by elimination, with no symbolized Qt-side stack.
- Fix verified: the "prod" variant (queued pattern + closing flag + waitForDone(), called after exec() and before the engine is dropped). Harness runs, 0 failures in: Debug 300 + 150 (toggle mode); ASan 150; Release 500 + 300 + 200 (scroll-only, 8 in parallel); Release with a real window (software scene graph) 40. That is 1640 harness runs. Real spike app with the fix: 400/400 clean (baseline 396/400), so 2040 clean runs in total. Everything is in crash-T-0041/results/summary-*.txt.
- Ablation: without the explicit pre-teardown call (only the provider destructor quiesces), the prod variant failed 2/300 times, both as an exit hang after main returned. The explicit call between exec() and engine drop is what removes it.
- About 0xe9f73: the disassembly is `mov rax,[rcx+20h]` after the QObjectPrivate::ConnectionData refcount release in the signal-emission tail, with rcx loaded from [obj+0x60]. That is a read through freed QObject private or connection data. That RVA never appeared in any harness or real-app run. The early crashes occurred in benchmark runs that end with Qt.quit(), the same teardown window, so crash B's class is the most likely explanation, but it is not proven.
- ASan is weak evidence: the Qt DLLs are not instrumented, so it found nothing. The Debug CRT heap and the Release builds did the work.

options compared:
- queued-signal pattern alone (T-0002 final / Qt doc example) — not enough — crashes at teardown with tasks in flight (about 1-3%; 4/400 in the real app).
- mutex-guarded target pointer with a plain QRunnable (guarded) — no better — same teardown crash (8/300); also needs a lock in the hot path.
- QPointer delivery from the GUI thread (olda) — avoid — violates Qt's ownership model. Emitting finished() twice from cancel() (oldb) is fatal.
- queued-signal pattern + shutdown (prod) — RECOMMENDED — 0 failures in 2040 runs.
- synchronous QQuickImageProvider — control only — blocks the reader thread, not suitable for artwork decode.

licenses: no new dependency; MSVC ASan runtime from the VS install; nothing installed system-wide; no system, security or WER settings changed (crash dialogs suppressed per process with SetErrorMode).

risks:
- The original 0xe9f73 may be a third mechanism; keep the stress target as a regression gate and re-run it with Qt PDBs if it reappears in the field.
- The shutdown hook must be called by every app entry point; production needs a registry or a Drop guard.
- Teardown cost: without cancel/closing checks a scroll storm builds a backlog of thousands of tasks and exit takes minutes.
- The ablation exit hang suggests Qt's own thread shutdown ordering is fragile with in-flight work. Do not rely on destructors alone.
- Reconstruction of the first shim is hypothetical; early exit-0-without-completion cases in the harness were not individually explained (they disappeared once stdout flush and ENGINE_DESTROYED checkpoints were added).

draft ADR: spikes/T-0002-cxx-qt/crash-T-0041/ADR-draft.md

Recommended production pattern for ADR 0008:
1. A response is created, signalled, cancelled and deleted only on Qt's pixmap-reader thread. Worker threads never hold a pointer to it and never call into it, not even through QPointer.
2. The task is a QObject + QRunnable (autoDelete). The only worker-to-response channel is the task's queued signal done(QImage), connected with the response as the context object (Qt::QueuedConnection); emit done() is the last statement of run().
3. Exactly one finished() per response. cancel() only sets an atomic flag shared with the task; a cancelled task still emits done() with an empty image. Never emit finished() from cancel().
4. A bounded QThreadPool lives in a shared_ptr state held by the provider, the responses and the tasks, with a newest-request-first priority and a closing flag that tasks check before doing work.
5. Teardown: call tw_image_provider_shutdown() on the GUI thread after QGuiApplication::exec() returns and before the QML engine is dropped. It sets closing and calls waitForDone(); late requests complete empty without starting a task. The provider destructor repeats it.
6. Keep a stress target equivalent to crash-T-0041/run-batch.ps1 (random exit 200-900 ms, 1 pool thread, about 300 runs, Release) and re-run it on any change to the provider.

Commit cebff40 on t/T-0041-image-provider-crash. All processes closed. Untracked oldb crash files (~1.2 MB) left in the worktree's results\ (prune blocked by a safety check); minidumps deleted.

follow-up tickets suggested:
- P0: Port the image-provider pattern plus the shutdown hook into apps/tinwave-desktop and add the stress script as a dev-only check | size: M
- P1: Fold the findings into ADR 0008, docs/rules/qml-ui.md and docs/rules/testing.md | size: S
- P2: Install the Qt "Debug Information Files" component and re-run the harness for a symbolized stack | size: S
- P2: Nightly run of the harness once CI exists | size: S

questions for lead / PO:
- Should the Qt Debug Information Files component be installed on the dev kit?
- "Shutdown hook required before engine drop" contract, or an RAII wrapper that owns both the engine and the provider?

---
Lead (2026-11-02): accepted; addendum folded into ADR 0008 (decision 5 replaced) and qml-ui.md/testing.md (P-0123).
Production uses an RAII owner: the desktop app's UI runtime struct owns the engine and the providers and its Drop runs
every provider shutdown before dropping the engine (fields ordered + an explicit Drop), with a test that the order holds;
the explicit call after exec() stays as well. Qt PDBs: parked — install only if a Qt-side crash reappears (P-0124).
Port → T-0044 (P-0122). Nightly harness → parked until CI (P-0125).
