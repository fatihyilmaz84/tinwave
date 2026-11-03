HANDOFF T-0063 (round 2)
status: done
branch: t/T-0063-rate-restart-hardening   commit: 4a862da (on top of f353733)
summary: Both blocking findings are fixed in decoder.rs and tests/rate.rs only. A rate flush now polls at the 1 ms minimum for its first 30 polls, then backs off to 50 ms like a command's flush. `Rec::change_rate` no longer uses 4-frame callbacks with sleeps. The binary now takes about as long as main's under the same load.

acceptance criteria / review findings:
- [x] Blocking 1 (unbounded 1 ms polling after a failed `switch_device`):
  - `AckPoll { interval, fast_left }` plus `RATE_ACK_FAST_POLLS = 30`. `wait_for_ack` gives an Audible flush 30 timeouts at `FLUSH_WAIT_MIN`, then doubles 2, 4, 8, 16, 32, 50 ms. A command's flush backs off from its first timeout. A command resets the state to the start. The ack-received reset in `run` is `AckPoll::new()` too.
  - Docs updated to match: the module docs and the `FLUSH_WAIT_MIN` doc now say a stream that never acks costs about 20 wakeups per second, as before.
  - Unit test `decoder::tests::a_rate_flush_is_polled_at_the_minimum_for_a_while_and_then_backs_off` shows the cap.
  - Mutation (cap removed: `matches!(restart, Audible)` without `&& fast_left > 0`): the test fails. The file was restored and touched.
  - On a 15.6 ms Windows timer, 30 polls is about 0.5 s of polling at about 65/s, then the backoff. On a 1 ms timer it is about 30 ms.
- [x] Blocking 2 (change_rate pacing cost) — `Rec::change_rate` is rewritten:
  - Pre-buffer 12,000 frames instead of 40,000.
  - `set_rate`, then two `replace_after(QueueItemId(1), [])` round trips. Commands are taken in order, so the second edit's answer means the decoder's loop pass that raised the flush request is over.
  - Then 256-frame callbacks. A single callback holds the whole 240-frame fade-out, with no pacing sleep. The only sleep left is 100 µs while waiting for the decoder's restart (`real == 0`).
  - One edit alone was not enough. Under 16 parallel copies the decoder was preempted between answering the edit and raising the request, and the ring drained in microseconds ("ran dry before 4 of 4 restarts" in 12 of 32 copies). With the second edit: 16 copies × 3 rounds plus `cargo nextest run -p tinwave-playback` each round gave 48/48 copies passed and nextest 352 passed, external load about 99 %.
  - Restart phases now vary: each play adds `(i * 41) % 109` frames, up to one period of the 440 Hz sine.
  - `FADE_TOLERANCE` is now 0.4 % (was 0.5 %). With 256-frame callbacks the mutated margin got thin (0.51 %).
  - The `no_click_at_a_rate_step_...` test now takes about 1.5 to 3.5 s under load (it was 83 s under nextest).
  - Mutation `RATE_FADE_MS` 5 to 2 in engine.rs (reverted, touched): `no_click_at_a_rate_step_...` failed 3 of 3, at 0.72 %, 0.78 % and 0.63 % above the steady maximum. Unmutated it passed 5 of 5 (together with the click-checker test).
  - The synthetic `the_click_check_passes_a_5_ms_fade_and_fails_a_2_ms_one` still passes and fails as required at 0.4 %.
- [x] Back-to-back measurement, main's rate binary (65b8bc3) against the branch binary, alternating, same minutes, same machine. Another agent's CPU load kept the machine near 100 % throughout, so the numbers are noisy. Wall time per run, with the "finished in" time from the test harness in brackets. Main has 20 tests, the branch has 24.

| load | run | main | branch |
|---|---|---|---|
| none from me (machine at 100 % from others) | 1 | 10.5 s (7.6) | 5.0 s (3.4) |
| | 2 | 3.5 s (3.0) | 3.3 s (3.1) |
| | 3 | 2.9 s (2.5) | 3.2 s (3.0) |
| | 4 | 2.5 s (2.4) | 2.2 s (2.1) |
| 16 CPU burners added | 1 | 5.6 s (5.2) | 6.3 s (5.9) |
| | 2 | 6.8 s (6.4) | 6.6 s (6.1) |
| | 3 | 7.5 s (7.0) | 5.6 s (5.3) |
| | 4 | 5.3 s (5.1) | 8.2 s (7.7) |

  - The two are comparable; the branch is not slower in any consistent way. The branch does more: a 40-restart test and four extra tests.
  - The earlier 24 to 40 s per run is gone.
- Non-blocking: the rate.rs header is fixed. It no longer says no clock is read for an assertion. It says the drag test bounds flushes by `Instant` elapsed time, so a disabled throttle is caught on a machine that is not heavily loaded. The device-switch TrackFailed dedupe and CorruptPackets dedupe are left alone (T-0077).

checks: fmt pass (`cargo fmt --all --check` exit 0), clippy pass (`--workspace --all-targets -D warnings`, exit 0), `cargo build --workspace` pass (exit 0), tests 802 passed / 0 failed / 6 skipped (workspace nextest, exit 0), deny pass (exit 0)
  - `tinwave-desktop::image_provider cancel_storm_with_requests_in_flight_then_quit_exits` (T-0071 flake) failed once in an earlier workspace run and passed in 2 of 3 reruns; it is unrelated to tinwave-playback. The final full run was clean.
files changed (this round): crates/tinwave-playback/src/decoder.rs, crates/tinwave-playback/tests/rate.rs
signals added/changed: none
ui: none
docs updated: decoder.rs module docs and the `FLUSH_WAIT_MIN` doc (fast polls then backoff)
deviations from ticket: none new. No engine.rs or other file outside scope is touched in the commits; the 2 ms mutation was applied to engine.rs temporarily and reverted.

proposals:
- (unchanged from round 1: dedupe CorruptPackets after a rate restart; T-0076 for the dedicated throttle clock; stale `EngineConfig::clock` / `set_rate` rustdocs in engine.rs; harness timeouts that are load-dependent: tests/common/mod.rs 278/321/360)
- P: Treat a device-switch re-read like a rate restart for TrackFailed dedupe | why: reviewer's non-blocking finding; T-0077 | size: S

questions for lead:
- none. A scratch cleanup note: my temporary benchmark/stress exes and CPU burners are deleted or killed, and nothing of mine is still running.
