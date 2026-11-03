HANDOFF T-0063 (round 1)
status: done (one criterion is covered at unit level, not as an integration test: see "Rate restart reaches the first new audio without backoff delay" and "deviations")
branch: t/T-0063-rate-restart-hardening   commit: f353733
summary: The rate-flush wait no longer backs off. The throttle now ends on the monotonic clock or on the engine's injected clock, so rate.rs moves the harness `ManualClock` instead of sleeping 130 ms. A track that fails again on a rate restart is reported once per item, and one that cannot be reopened leaves as Failed. rate.rs has the relative click test (2 ms fade fails, 5 ms passes) and a deterministic drag test. Only decoder.rs and tests/rate.rs changed.

acceptance criteria:
- [x] A 2 ms fade is caught by the relative click test; a 5 ms fade passes — proof:
  - `rate::the_click_check_passes_a_5_ms_fade_and_fails_a_2_ms_one` checks the checker on synthetic dips: forty dips at different phases, so the phase-dependent excess is reached.
  - `rate::no_click_at_a_rate_step_the_largest_sample_step_is_the_steady_signals` runs the real engine: 40 restarts 2 % apart, the largest step within ±30 ms of a restart against the same recording's steady maximum, tolerance 0.5 %.
  - Mutation `RATE_FADE_MS` 5 to 2 in engine.rs (reverted, not committed): the real test failed 3 of 3 runs ("0.75 % / 0.91 % / 0.80 % above the steady signal's", all over the 0.5 % tolerance). Unmutated it passes.
  - The old large steps (1.25, 0.75, 1.5, 1.0) are kept as `no_click_at_a_large_rate_step_...` with a 3 % relative tolerance (see deviations).
- [x] Rate restart reaches the first new audio without backoff delay (simulated device) — proof: unit test `decoder::tests::a_rate_flush_is_polled_at_the_minimum_and_a_commands_backs_off` on the new `Decoder::wait_for_ack`. It checks that a rate flush never leaves `FLUSH_WAIT_MIN`, a command's flush still goes 2, 4, 8, 16 ms up to 50 ms, a command resets it, and a disconnect ends the thread.
  - Mutation (rate flush also backs off): this test fails.
  - I first wrote a wall-clock integration test (ack 70 ms late, time to the first new audio). Restoring the backoff did not make it fail (21 to 24 ms against a bound of 20 ms, once 8.7 ms), because of timer granularity and phase. A test that cannot fail on the old code is not a test, so I removed it. There is no wall-clock integration test.
- [x] rate.rs has no real sleeps for the throttle, runs faster, stable under parallel copies — proof:
  - Every 130 ms sleep is now `h.clock.advance(PAST_THROTTLE_MS)`.
  - `a_drag_costs_one_flush_per_interval_and_ends_on_the_final_value` is rewritten around the manual clock: two 48-step drags of 2 ms each, plus 30 ms to close each. It asserts 3 flushes at least and `1 + 2 + real_elapsed/125` at most, a bound a slower machine only widens. Mutation (throttle off): 13 flushes against at most 3, 3 of 3 runs, so it fails.
  - On an idle machine (7 % CPU) the whole binary runs in 2.7 to 3.9 s.
  - 16 copies of the rate binary plus `cargo nextest run -p tinwave-playback` at the same time, 3 rounds: 48/48 copies passed and nextest 352 passed each round.
  - 24 copies, same setup: 72/72 and 92/96 in rounds where the machine was quiet. In rounds where other builders were using the CPU, failures were all in the shared harness's bounded 3 s waits (`tests/common/mod.rs:278/321/360`, so `settle`, `wait_buffered`, `expect_edit_applied`), which I cannot touch. The unchanged main-branch rate binary fails the same way: one run had 23/24 copies fail, another 2/24. I did not run main and the branch under identical load, so I cannot say they fail equally often.
  - Earlier load failures that were mine are fixed:
    - A hard cut at a restart because the ring ran dry while a starved decoder had not yet seen the request. `Rec::change_rate` now uses 4-frame callbacks, waits for 40,000 buffered frames first, and marks a restart `starved` if the ring ran low. `click_free` leaves those out and requires at least half to remain.
    - The drag test's per-step edit round-trips timed out under load; it now syncs every 8th step.
- [x] A track failing again on restart reports one TrackFailed and leaves as Failed — proof:
  - `rate::a_track_that_breaks_again_at_the_same_place_after_a_rate_restart_is_reported_once`: BreakingOpener breaks at the same byte on the re-read; asserts one `TrackFailed` and `TrackLeft{Failed}`. Mutation (dedupe off): fails, with two `TrackFailed` in the event list.
  - `rate::a_track_that_cannot_be_reopened_by_a_rate_restart_fails_once_and_leaves_as_failed`: `VanishingOpener` fails every open after the first; asserts one `TrackFailed`, `TrackLeft{Failed}`, then `QueueEnded`. Mutation (`heard_failed` off): fails with `cause: Finished`, which was the old behaviour.
  - Unit test `decoder::tests::an_item_is_reported_failed_once_and_the_heard_one_leaves_failed` covers the same two behaviours without an engine.
- [x] The T-0066 item (one flush for 48 steps at 100 % CPU) is gone — the drag test no longer depends on how fast the decoder reacts, and the click test excludes starved restarts (see above).
- [x] fmt, clippy, nextest, deny pass — fmt pass, `cargo clippy --workspace --all-targets -- -D warnings` pass, `cargo nextest run --workspace` 802 passed / 6 skipped (exit 0), `cargo deny check` exit 0.

checks: fmt pass, clippy pass, tests 802 passed / 0 failed / 6 skipped (workspace), deny pass
files changed: crates/tinwave-playback/src/decoder.rs, crates/tinwave-playback/tests/rate.rs
signals added/changed: none
ui: none
docs updated: decoder.rs module docs only (rate flush polling, throttle clocks, restart failure rules)

deviations from ticket:
- Hybrid throttle clock instead of a purely injected one. The decoder already holds `Decoder::clock`, which is the harness's `ManualClock`, so no engine.rs change was needed. A flush is allowed when 125 ms have passed on the monotonic `Instant` or on that injected clock. A wall clock set back never holds a change up.
  - Why not injected only: I built it that way first. Two tests outside my scope then failed deterministically: `rt_alloc::render_never_allocates_across_rate_changes` (sleeps 130 ms against a frozen manual clock, then waits about 150 s) and `controller_signals::a_heard_play_has_one_rate_session_with_the_time_weighted_rate_of_its_schedule`. The hybrid keeps both, and all other existing tests, exactly as they were (workspace 802 passed).
  - Production is unchanged in practice, since both clocks are wall time there.
  - Cost: rate.rs asserts upper bounds that widen with real elapsed time, not exact equalities, which hold on any machine.
- `no_click_at_a_rate_step_...` now steps the rate by 2 % (40 restarts), not 25 % to 50 %. At large steps the phase vocoder's ripple scatters by about 1.5 % per restart, which is as big as a 2 ms fade, so no relative bound could both catch a 2 ms fade and never flake. The large steps remain in `no_click_at_a_large_rate_step_...` at 3 % relative; a hard cut there is a step more than ten times larger.
- Test count: the old drag test is replaced; two new click tests, one checker test, and four failure/throttle tests were added. rate.rs has 24 tests.
- Corrupt-packet reports are not deduplicated, only `TrackFailed`. A restart re-reads from the audible frame and can count some already-reported packets again, but telling them apart needs per-packet positions from the reader. See proposals.
- A rate flush's ack wait now polls at 1 ms with no cap, as the ticket says. If a stream never acks (dead device, no switch yet), the decoder wakes about 1000 times a second until something else happens. A command's flush keeps its backoff.
- I did not edit engine.rs or any file outside scope.

proposals:
- P: Dedupe CorruptPackets after a rate restart | why: a restart re-reads the item from the audible frame and the reader can count packets that were already reported; a count with the packet's source frame (or a reader-side "reported up to frame N") would let `drop_reader` send only new ones | size: S | evidence: decoder.rs `drop_reader`, `reader.take_corrupt_packets`
- P: Own the throttle clock in EngineConfig | why: the throttle uses the event `Clock` (wall time) only because `EngineConfig.clock` is the sole injection point in scope; a `RateThrottle`/monotonic clock field would remove the hybrid and make rt_alloc.rs and controller_signals.rs use `clock.advance` instead of 130 ms sleeps | size: S | evidence: decoder.rs `throttle_wait`, tests/rt_alloc.rs:340
- P: Reword the stale rustdocs in engine.rs | why: `EngineConfig::clock` says "audio positions never use it" and "wall-clock source stamped onto events" (engine.rs:93), but the decoder now also reads it for the rate throttle; `set_rate` still quotes the spike's "about 15 ms dip, 10 to 20 ms from call to first audio" (review-1 non-blocking finding), which predates pre-roll | size: S | evidence: engine.rs:93, engine.rs:465-470 (T-0066 owns engine.rs now)
- P: Decouple the harness timeouts from CPU load | why: `settle`, `wait_buffered`, `expect_edit_applied` give up after about 3 s of 100 µs sleeps; under heavy external load (other builders) otherwise correct tests fail there (main's rate binary does too) | size: S | evidence: tests/common/mod.rs:278, 321, 360

questions for lead:
- Do you want the pure injected-clock design instead of the hybrid? That means editing `rt_alloc.rs:340` and the `controller_signals.rs` rate tests to advance the rig clock instead of sleeping, which are outside this ticket's scope; the throttle-clock proposal above is the clean way.
- Is the rate flush polling at 1 ms with no cap acceptable for a stream that never acks (decoder wakes about 1000 times a second until a command or device switch)? I followed the ticket literally.
