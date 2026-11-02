HANDOFF T-0028 (round 1)
status: done
branch: t/T-0028-engine-pairing-fixes   commit: 517b533 (on top of main 219ed37)
summary: Both blocking pairing bugs from the T-0016 round-2 review are fixed, the load-dependent test hang is gone, and the spurious-Underrun window on device switch is closed. The seeded property test also found a third bug, which is fixed. Scope was crates/tinwave-playback/ only.

acceptance criteria:
- [x] The reviewer's two repros are tests and pass; they fail on the pre-fix code — tests/pairing.rs (`seek_after_the_queue_ended_is_rejected_and_nothing_is_emitted`, `failed_switch_device_does_not_start_the_track_twice`, `seek_is_rejected_after_stop_and_before_anything_is_loaded`). Pre-fix: both reverted ⇒ both repros and the property test fail; only seek fix reverted ⇒ `seek_after..` and the property test fail (seed 1); only switch fix reverted ⇒ `failed_switch..` and the property test fail (seed 5, "switch refused"); fixed code ⇒ all pass.
- [x] Property test passes and catches both bugs on pre-fix code. It also caught a third bug: `switch_device` after the queue ended with a failing track appended re-opened the old last track and emitted TrackLeft without TrackStarted (seed 8 with both halves of the fix reverted). Default run seeds 1..32.
- [x] Stress: 9 playback test executables × 6 copies × 6 rounds = 324 processes, 0 failures; × 10 copies × 6 rounds = 540, 0 failures (864 total). Script /tmp/stress.sh, not committed.
- [x] Zero callback allocations still hold (rt_alloc passes; Vec-in-render mutation fails with 766). fmt, clippy `-D warnings`, tests, deny pass.

checks: fmt pass, clippy pass, tests 227 passed / 0 failed (workspace), deny pass

changes (crates/tinwave-playback):
1. `Engine::seek`: rejected with `NothingLoaded`, emits nothing, when no track is current (queue ended, `stop`, nothing loaded). Chosen over restart-with-`TrackStarted{Requested}` (restarting a finished track is a `load`). Documented on `Engine::seek`.
2. `Engine::switch_device`: the sent `Reconfigure` is recorded with `command_sent` on the error path; a failed `send` (Busy) emits `OutputLost`. With no current track a switch resumes nothing; decoder `Reconfigure{item: None}` keeps `st.cur` so appended-but-unheard tracks still play (third bug).
3. tests/failures.rs: mid-decode and last-track loops use new `Harness::run_until` (bounded render+poll). Remaining `wait_buffered` calls are in tests whose queue still has audio, or a single-frame check after a seek to the last frame.
4. tests/pairing.rs property test: ops load, append, seek, stop, pause/resume, switch ok, switch refused (then forced retry), play N blocks, play to end; pool: good tracks, missing file, garbage file, WAV cut mid-stream (`BreakingOpener`); `SeededRng` seeds 1..32 × 36 steps (~11 s); `TINWAVE_PAIRING_SEEDS=N` for more; `assert_paired` (allow open) and position monotonicity after every block and step, all-closed at the end; failure prints seed and op trace. `BrokenSource`/`BreakingOpener` moved to common.
5. Per-renderer `seen_data` replaces `Shared::primed` (set on first audio taken, cleared on flush ack; underruns count only after data was seen; decoder and `switch_device` no longer touch it). Three renderer unit tests. Consequence: a stalled first read after a load reports no Underrun ("buffering"); a stall after data was seen still reports.

signals added/changed: none. Event contract tightened: `seek` rejection emits nothing; failed `switch_device` emits `OutputLost` only. T-0024 can rely on exactly one `TrackLeft` per `TrackStarted` and no `Seeked` without a current track.
deviations from ticket: none

proposals:
- P: CI job running the playback test binaries concurrently (the 864-process script) | size: S

questions for lead: none
