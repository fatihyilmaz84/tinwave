HANDOFF T-0049 (round 1)
status: done
branch: t/T-0049-artwork-provider-followups   commit: 15f7786
summary: The gate flake is fixed first: the test measures requests in flight at the moment the event loop ends (new `in_flight_at_quit` counter in `image-stress`), not from the final counters after the quiesce, with a bounded retry. The provider follow-ups are done: Rust owns the max-edge limit and passes it to C++; refused and no-artwork responses report an error string, so QML gets `Image.Error`; a one-edge `sourceSize` keeps its edge. All gates green; 400 Release stress runs and 50 flake-proof runs under parallel load, no failures.

Root cause of the flake: `finished < responses` was read after `shell.shutdown()`. After the quiesce, the queued tasks drain within microseconds; whether the reader thread had delivered their `finished()` by the time the counters were read depended on CPU load. Seen by hand without load: one of three scroll runs ended with `finished == responses` while `in_flight_at_quit` was ~2340 in all three. The test now asserts per mode that some run had `in_flight_at_quit > 0`: storm 4 listed cases, scroll 3; a mode with no qualifying run gets at most 6 extra attempts, then fails. Every run still exits cleanly with `duplicate_finishes=0`, so the assertion is no weaker.

acceptance criteria:
- [x] Oversize → `Image.Error`, no task run; no-artwork key → `Image.Error`; in-range → `Image.Ready` — `image_provider::refused_and_missing_artwork_are_image_error_and_served_sizes_are_image_ready`: 48x48 and 1024x8 Ready at exact size; 2000x2000, 1025x10, 10x1025 and `missing-1` Error. Counters: responses 6, finished 6, `tasks_run` 3, `requests` 3, `misses` 1. Qt warnings carry "…2000x2000 is above the limit of 1024 px per edge" and "no artwork for \"missing-1\"…".
- [x] Changing the Rust limit changes the C++ refusal — `image_provider::the_edge_limit_comes_from_rust_not_from_a_second_liter…`: `--max-edge=32` refuses 48x48, serves 32x32; default refuses 1500x8; `--max-edge=2048` serves 1500x8. Unit test `provider_core::the_edge_limit_is_configurable_and_clamped` (clamp 1..i32::MAX). `kMaxEdge` gone; `git grep` finds no 1024 in `cpp/`.
- [x] 64x0 and 0x64 → 64-edge image — `image_provider::a_one_edge_source_size_keeps_that_edge`: both Ready at 64x64; 0x0 → 128x128; 64x32 stays 64x32.
- [x] Stress >= 300 Release runs — `run-stress.ps1`, offscreen, Release, after the C++ changes: 400 runs, crashes=0 hangs=0 bad_output=0 (the script fails a run on `duplicate_finishes != 0`): storm pool 1 quit 200-900 ms 4 parallel 200/200 (3,026,087 responses, 2,972,367 cancels, 24,375 tasks, 257 s); scroll pool 2 quit 50-900 ms 4 parallel 100/100 (248,575 responses, 27,292 tasks, 22 s); storm pool 2 quit 50-900 ms 4 parallel 100/100 (1,365,231 responses, 23,791 tasks, 79 s). `target\stress-failures` empty; no processes left.
- [x] fmt, clippy -D warnings, tests, deny.
- [x] P-0169: `cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` 50 times in a row 10:17:30–10:25:33 while `cargo nextest run --workspace` ran back to back in another process (14 rounds from 10:17:20; each 499 run, 499 passed, 6 skipped, ~36 s). A first attempt (50/50) overlapped only partly, so it was redone with timestamps.

ADR 0008 lifetime rules 1-6 unchanged and holding: reader thread owns the response; only the queued signal and the atomic flag cross; exactly one `finished()`, only from `complete()`; gate and closing path untouched; `done(QImage, QString error)` carries no pointer to the response; the error string is set on the reader thread inside the same queued slot that calls `complete()` (or before the queued refuse path); quiesce ordering untouched; cancelled/closing outcome is empty image + empty error, so those stay silent.

checks: fmt pass, clippy pass, tests 499 passed / 0 failed / 6 skipped (nextest --workspace, 36 s), deny pass.

files changed (apps/tinwave-desktop/): cpp/image_provider.cpp, cpp/image_provider.h, cpp/thumb_task.h; src/app/artwork_bridge.rs; src/app/wiring.rs; src/image/provider_core.rs; tests/image_provider.rs; tools/stress/main.rs; tools/stress/Sizes.qml (new)
signals added/changed: none
ui: none (qml/ untouched; Sizes.qml is a stress fixture)
docs updated: none (ADR 0008 and qml-ui.md out of scope; proposal 1)

deviations from ticket:
- `src/app/wiring.rs` (outside scope): three-line edit — `ShellConfig.artwork_max_edge`, set to `image::MAX_THUMBNAIL_EDGE` in `production()`, passed to `ArtworkProvider::install` (new `max_edge: u32` parameter). Lets the stress binary take `--max-edge=`.
- One-edge `sourceSize` becomes a square of that edge, not a source-derived aspect: the aspect is only known after the Rust call on a pool thread and the reader thread must not wait (the ticket's "or square when unknown"). Documented in `resolveSize`, `ArtworkProvider::install` docs and the test.
- `image-stress` gets `--max-edge=`; its `STRESS_DONE` line gets a last field `in_flight_at_quit` (run-stress.ps1's regex reads only earlier fields).

proposals:
- P: Add the T-0049 notes to ADR 0008 and qml-ui.md (limit owned by Rust, passed at install, no C++ literal; refused/no-artwork → `Image.Error` with a message, cancelled/closing silent; optionally: a task must never be the last holder of the shared state) | size: S
- P: Quiet the per-image QML warning for no-artwork covers before the library source lands (thousands of log lines otherwise) | size: S | evidence: Storm.qml:50
- P: Optional "aspect of key" query on `ArtworkSource` so a one-edge `sourceSize` keeps a non-square aspect (not needed for square covers) | size: S

questions for lead: none. Scratch scripts in the session scratchpad only; worktree clean; no processes running; the worktree's target/ (6.3 GB) can be deleted after merge.

---
Lead (2026-11-03): wiring.rs edit accepted (minimum, forced by the install signature). Square-for-one-edge accepted.
P-0177 (ADR/qml-ui notes) → lead applies on merge; P-0178 (quiet no-artwork warnings) → T-0052 note for the
library-backed artwork source (parked until that ticket exists, recorded on P-0142's outcome); P-0179 (aspect query)
parked.
