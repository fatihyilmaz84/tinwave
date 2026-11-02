HANDOFF T-0044 (round 2)
status: done
branch: t/T-0044-artwork-image-provider   commit: fd123af
summary: Fixed the blocker (`default-run`) and all seven items listed. `cargo run -p tinwave-desktop` opens the "Tinwave" window again (confirmed, then killed). The closing check and `pool.start()` are now atomic against `quiesce()`. All gates are green, and 460 further Release stress runs had 0 failures.

acceptance criteria (unchanged from round 1, re-verified):
- [x] UiOwner quiesces providers before the engine (order asserted) — 3 `ui_owner` unit tests pass.
- [x] Headless cancel storm + quit with tasks in flight — `image_provider::cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` passes, with the stronger assertion in item 6.
- [x] Stress >= 300 Release runs, 0 crashes, 0 hangs — round 2, after the C++ changes, 460 runs, offscreen, `run-stress.ps1`: storm pool 1 quit 200-900 ms 4 parallel 200/200; storm pool 2 quit 50-900 ms 4 parallel 100/100; scroll pool 1 quit 100-900 ms 4 parallel 100/100; storm pool 1 quit 200-900 ms sequential 60/60. crashes=0 hangs=0 bad_output=0. tasks run: 22,535 + 22,813 + 11,128 + 9,086.
- [x] One `finished()` per response — `every_response_gets_exactly_one_finished_and_the_cache_serves_repeats` (64/64, duplicates 0); `duplicate_finishes=0` in all 460 runs.
- [x] Gates — see checks.

round-2 items:
1. `default-run = "tinwave-desktop"` in `[package]`; `cargo run -p tinwave-desktop` started the app, window "Tinwave" appeared, process killed.
2. `ProviderShared` has a `gate` mutex. The `ThumbResponse` constructor does "check closing + `pool.start()`" under `gate`; `quiesce()` sets `closing` under `gate`. After `quiesce()` set the flag no task can start, so `waitForDone()` really waits for the last one. If closing is already set, the unstarted task is deleted by a `unique_ptr` and the response completes empty through the queued path. Structure comment (rule 4) and `thumb_task.h` comments updated.
3. C++ refuses sizes above `kMaxEdge = 1024`: the response completes empty, no `QImage` allocated. Width or height <= 0 falls back to the default 128 square. `ThumbTask::run` skips the Rust call if `img.isNull()`.
4. Removed `process::exit` altogether instead of quiescing before it: `on_object_creation_failed` can't capture the provider handle (the handler requires `Send`; `Rc`/`UniquePtr` aren't). The handler records the failure in an `Arc<AtomicBool>` and calls `engine.exit(2)`, which ends a running loop (async case). `Shell::exec` returns `EXIT_QML_LOAD_FAILED` (2) without entering the loop if the failure is already known (synchronous qrc case), and returns 2 after the loop if the flag was set. Teardown is then the normal one: providers quiesced, engine dropped, exit code 2. T-0040's `missing_root_qml_exits_nonzero_with_a_message_instead_of_hanging` still passes (exit 2 plus the stderr message).
5. `ui.add_provider(...)` right after `ArtworkProvider::install`, before the failure handler and `load()`. A small `engine_of(&mut ui)` helper replaces the held engine borrow.
6. The test tracks, per mode, how many runs quit with `finished < responses`, and requires at least one such run in storm mode and one in scroll mode.
7. Comments: `~ProviderShared` comment reworded and its `waitForDone()` removed (a task holds a reference, so none can be running when the destructor runs; `QThreadPool`'s own destructor still waits). `provider_core.rs` says "the `Image`s of the future artwork component". `lib.rs` says unsafe is allowed only in `app`.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 417 passed / 0 failed (nextest --workspace, 6 skipped), deny pass. First full nextest run had 1 failure: `tinwave-db::storage_growth ninety_days…` took 215 s against its 60 s wall-clock budget (the known T-0047 issue; the immediate re-run passed 417/417, 58 s). The 21 `tinwave-desktop` tests pass on their own.

files changed (apps/tinwave-desktop/): `Cargo.toml`, `cpp/image_provider.cpp`, `cpp/thumb_task.h`, `src/app/wiring.rs`, `src/image/provider_core.rs`, `src/lib.rs`, `tests/image_provider.rs`
signals added/changed: none
ui: none
docs updated: none this round. ADR 0008 atomic-closing rule for the lead: "closing flag set under a mutex in `quiesce()`, and checked plus `pool.start()` under the same mutex in the response constructor", as in `cpp/image_provider.cpp`.
deviations from ticket: round-1 deviations stand (bridge file in `src/app/`, `[[bin]] image-stress`, small edits outside scope). New: item 4 as described.

proposals:
- P: Bind `QCoreApplication::exit/quit` in cxx-qt-lib or a shim | why: the load-failure path needs the engine's exit signal plus a flag because a pre-`exec` exit request is ignored | size: S | evidence: `src/app/wiring.rs` (`Shell::exec`)

questions for lead:
- None. No `image-stress` or `tinwave-desktop` processes left running.

---
Lead (2026-11-03): item 4's approach (no `process::exit`; flag + `engine.exit(2)`; normal teardown) accepted — better
than the literal ask. Branch is from b76192d; main has T-0047's speed check, so the storage_growth wall-clock failure
won't recur after merge. Proposal → P-0149 parked (the current flag + exit works; revisit if another path needs a
pre-exec quit).
