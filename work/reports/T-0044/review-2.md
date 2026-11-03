REVIEW T-0044 (round 2)
verdict: APPROVE
checks (run by reviewer, worktree at fd123af, Qt 6.12 env): fmt pass (exit 0). clippy pass (`--workspace --all-targets -D warnings`, exit 0). tests pass: `cargo nextest run --workspace` 417 passed, 6 skipped, 0 failed in 34 s (incl. all 3 startup/image_provider integration tests and the ui_owner, cache, provider_core unit tests). deny pass.
stress (reviewer, Release `image-stress`, offscreen, `run-stress.ps1`): 200 runs, 0 crashes, 0 hangs, 0 bad output.
- 100 runs: storm, pool 1, quit 200-900 ms, 4 in parallel: ok=100 in 77 s; 1,601,774 responses, 1,573,833 cancels, 14,731 tasks run.
- 60 runs: scroll, pool 2, quit 50-900 ms, 4 in parallel: ok=60; 17,157 tasks run.
- 40 runs: storm, pool 2, quit 50-900 ms, sequential: ok=40; 9,257 tasks run.
- Afterwards no image-stress/tinwave-desktop process left, no `target/stress-failures`, worktree clean.
merge: main b3795ba; `git merge-tree --write-tree main t/T-0044-artwork-image-provider` clean (tree afd5e91).

acceptance criteria:
- [x] UI owner quiesces providers before the engine (order asserted): `ui_owner::tests::drop_quiesces_every_provider_before_the_engine`, `explicit_shutdown_does_not_drop_the_engine_and_drop_repeats_it_before_the_engine` pass; provider registered right after `install` (wiring.rs:123-129).
- [x] Headless cancel storm + quit: `cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` (10.7 s), now requires >= 1 storm and >= 1 scroll run quitting with `finished < responses` (:188-199). By hand the margins are large (scroll 700 ms: 2257/3710, 2522/3625, 2432/3702; storm 800 ms: 6618/23540) — not flaky.
- [x] Stress >= 300 Release runs: builder 460 this round; reviewer 200 this round + 350 in round 1; all 0 crashes, 0 hangs.
- [x] No pointer to a response leaves the reader thread; exactly one finished(): cpp/image_provider.cpp:103-157. The unstarted task is deleted by `unique_ptr` on the reader thread where it was created; refuse and closing paths complete through a queued `invokeMethod`; `complete()` is still the only `finished()` emitter. Settle 64/64, duplicates 0; `duplicate_finishes=0` in all 200 runs.
- [x] fmt, clippy -D warnings, tests, deny.

Round-2 focus items:
1. `default-run` (Cargo.toml:9-10): `cargo run -p tinwave-desktop` started `target\debug\tinwave-desktop.exe`, window "Tinwave" appeared (pid 5000), killed, none left.
2. Gate: `quiesce()` sets `closing` inside a scoped `lock_guard` (:63-66); `waitForDone()` runs after release (:67). The constructor checks `closing` and calls `pool.start(task.release(), …)` under the same `gate` (:119-130); when closing, the `unique_ptr` deletes the task (and its connection) and the response completes exactly once via the queued path (:131-132). Lock order: `gate` is a leaf lock except QThreadPool's internal mutex taken inside `start()`; the GUI thread never holds `gate` across a wait; workers never take it; the reader thread drops its own mutex before `processJob` — no cycle. `waitForDone()` waits until the queue is empty and no thread is active, and Qt deletes autoDelete tasks before decrementing `activeThreads`, so no task exists after the first quiesce.
3. Size limits (scratch QML through Release `image-stress`): 2000x2000, 1025x10, 10x1025 completed with no task (tasks_run=5 of 8, finished=8), empty image; no sourceSize, 64x0, 0x64 fell back to 128x128; 1024x1024 and 48x48 served at size. `isNull()` guards `bits()` (:85-91).
4. Load failure (debug binary, `TINWAVE_MAIN_QML`): `qrc:/nope/Main.qml` (sync) exit 2 with the stderr message; `file:///C:/nonexistent/Main.qml` exit 2; `http://127.0.0.1:9/Main.qml` (async, loop running) exit 2 — `engine.exit(2)` really ends a running loop. `exec` returns 2 before `app.exec()` when the flag is set (wiring.rs:164-178); teardown is the normal `shutdown()` and drop. `git grep process::exit` under apps/ finds only the explanatory comment. T-0040's load-failure test passes. No exit path skips the quiesce.
5. `engine_of` (wiring.rs:192-199) sound: each `Pin<&mut>` borrows `ui` and ends before `add_provider` or the move into `Self`; only failure is a null engine, on the first call, before `install`.
6. Per-mode assertion present, large margins.
7. `~ProviderShared` without `waitForDone()`: every `ThumbTask` holds `m_shared` from construction to deletion, `run()` reaches `rustCtx` only through it; no task path runs without that reference. `QThreadPool`'s own destructor still waits (comment :56-57 accurate).

blocking findings:
- none

design check: n/a (qml/ untouched; tools/stress scenes are fixtures)

non-blocking findings:
- cpp/image_provider.cpp:46 — `kMaxEdge = 1024` duplicates `MAX_THUMBNAIL_EDGE` (src/image/provider_core.rs:21), linked only by a comment; could drift. No test exercises the C++ refusal.
- cpp/image_provider.cpp:108-109 — a sourceSize with one dimension (64x0) becomes a 128 square instead of keeping the aspect from the given edge, as Qt normally does. Matters once the real artwork component sets one dimension.
- Refused, missing and cancelled responses complete with a null texture factory and no `errorString()`: QML sees `Image.Ready` at 0x0, not `Image.Error`. Harmless now; the artwork component may want `errorString()` for a placeholder by status.
- Latent, unreachable today: if a task were ever the last `shared_ptr` holder, `~ProviderShared` (and `~QThreadPool`) would run on a pool thread and wait on that pool. The quiesce-then-drop-handle order prevents it; worth one line in the ADR 0008 addendum for future providers not owned by `UiOwner`.

proposals:
- P: Pass the max-edge limit to the C++ shim through `tw_provider_handle_new` (or a shared constant / static assert) instead of a second literal | why: the limits cannot drift | size: S
- P: Refused or no-artwork responses set `errorString()` | why: QML gets `Image.Error` and can show the placeholder state | size: S

notes for lead: ready to merge. On merge add to ADR 0008: the atomic-closing rule (closing set under a mutex in `quiesce()`, checked plus `pool.start()` under the same mutex in the response constructor) and that the handle must outlive every task (the quiesce ordering guarantees it). Packaging exclusion for `image-stress` stays on T-0042. All processes gone; only scratch file: scratchpad\Sizes.qml.
