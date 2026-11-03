HANDOFF T-0027 (round 1)
status: done
branch: t/T-0027-rate-control-engine   commit: dfa60b9 (3 commits on top of e46a240; nothing pushed)
summary: The decoder thread now stretches at the file rate (new `rate_stage.rs`, then the T-0019 resampler), bypassing the stretcher at exactly 1.0. A rate change is a throttled flush-and-restart at the audible source position. The renderer plays a 5 ms fade-out and fade-in and publishes `eff_rate`, `wall_frames` and `src_micro`. `Engine::set_rate` emits `EventKind::RateChanged`; the controller maps it to the `RateChanged` signal and gets `set_rate`. The P-0206 `BlockPlan` helper and the P-0209 two-sided onset test are in.

acceptance criteria:
- [x] Zero callback allocations across rate changes — proof: `rt_alloc::render_never_allocates_across_rate_changes`. It covers 1.25, 0.75, 1.5, 1.0 and 1.1, plus a stretched seek, a change while paused, a device switch and the track end. Mutation: an allocation inside the fade-out path failed only this test.
- [x] Position accuracy through 4 rate changes ≤ 3 ms mean on a click train — proof: `rate::position_accuracy_through_four_rate_changes_is_within_3_ms_on_a_click_train` (48 kHz file) and `position_accuracy_holds_when_the_file_has_another_rate_than_the_stream` (44.1 kHz file, so stretch + resample). Mutations: `Mark::source_frame_at` ignoring the rate, and the pre-roll compensation removed from the discard. Both failed these tests. I did not print the measured means, only the ≤ 3 ms assertion. Clicks within 50 ms of a restart are excluded.
- [x] Time-weighted rate over 1.0→1.25→0.75→1.5→1.0 within 0.1 % — proof: `rate::time_weighted_rate_matches_the_schedule_within_a_tenth_of_a_percent`. It uses 2 s per step and acknowledges flushes with empty callbacks, so every segment is exactly 96,000 frames. It asserts the engine's rate against the nominal schedule within 0.1 %, and the engine's position against `src_micro` within 10 frames. The counters are covered by the renderer unit tests. Mutation: `src_micro` booked at rate 1.0 failed it.
- [x] No click at a rate step — proof: `rate::no_click_at_a_rate_step_the_largest_sample_step_is_the_steady_signals`. Mutation: no fade-out failed it. Deviation: the bound is 1.03× the steady sine's step, not 1.00×. A stretched sine's own steepest step is up to 2 % above a pure sine's even with no restart, measured over 109 start phases in a temporary probe. ADR 0006's "equals" was a rounding of 0.033 vs 0.031. The fade shapes are asserted exactly in the renderer unit tests. Stable 24/24 with 24 copies of the test binary running at once.
- [x] Bypass at 1.0 is bit-exact with the un-stretched path — proof: `rate::bypass_at_one_is_bit_exact_after_a_trip_through_the_stretcher`. After a trip through 1.25 and back to 1.0, every sample after the 5 ms fade-in equals the file's bit for bit, and the fade-in is exactly a ramp. Mutation: routing 1.0 through the stage failed it.
- [x] P-0206: `stretch::BlockPlan` (public, `BLOCK_FRAMES = 1024`) is the production rate arithmetic. It has 6 unit tests and a doctest, and tests/stretch.rs `drive()` now uses it.
- [x] P-0209: `stretch.rs::the_onset_lands_at_the_source_position_over_and_under_discarding_both_fail`. Leading silence of 4000 frames, onset within ±128 frames of S/rate at 0.75, 1.25 and 1.5. It also asserts that a lag 256 frames too long or too short lands outside the tolerance, on its own side.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 675 passed / 0 failed (workspace nextest; 6 skipped as before), deny pass (advisories, bans, licenses, sources ok). Also: doctest passes, a second `cargo build -p tinwave-playback` compiles nothing, `pairing::random_command…` passed with 200 seeds including rate ops, and `rate.rs` passed 24/24 with 24 copies of the binary running at once.

files changed (all under crates/tinwave-playback/):
- new: src/rate_stage.rs, src/stretch/plan.rs, tests/rate.rs (20 tests)
- src: decoder.rs, engine.rs, event.rs, lib.rs, reader.rs, renderer.rs, shared.rs, stretch/mod.rs
- src/controller: mapper.rs, mod.rs, runtime.rs
- tests: stretch.rs, rt_alloc.rs, controller_signals.rs (6 new tests, plus Speed disabled in the existing "constructs nothing" test), controller_random.rs and pairing.rs (rate ops in the random walks)

signals added/changed: none new. `RateChanged` (T0) is now emitted. It goes through the existing 600 ms coalescer with first-from and last-to, needs a session like `VolumeChanged`, and is not built when the Speed category is off (tested). `RateSession` (derived) is not emitted here. `Engine::rate_stats()` / `Controller::rate_stats()` and `RateStats::time_weighted_rate_since` give the controller what it needs. No size change.

ui: none
docs updated: none. docs/ is outside the ticket scope. The lead should update:
- signal-catalog §2: replace the "license check" sentence with Signalsmith, MIT, 0.75–1.5, bypass at 1.0, `RateSession` from the engine's `src/wall` counters.
- architecture.md: stretch and resample run on the decoder thread, the callback only fades and counts.
- spec §7.
- ADR 0006: the three points under "deviations".

deviations from ticket / ADR 0006:
1. **No segment queue.** A rate change is a flush, so the ring only ever holds one rate. The decoder stores `Shared::stream_rate` while the ring is empty, after the renderer's ack and before the first push. The renderer reads it after popping and books `src_micro`, `eff_rate` and `consumed` (= `wall_frames`). The rate is in stream frames per output frame, so the file-rate-to-device-rate factor from ADR decision 6 is not needed. Counters are exposed as `RateStats`.
2. **Position uses `Mark.rate`, not renderer-derived positions.** Marks carry the rate (`source_frame_at`). A rate restart appends a `MarkKind::Rate` mark and keeps the older marks, because the control side still resolves positions in the device buffer through them. Marks after `consumed` are dropped. Tests with 2,400- and 4,800-frame device latency cover a continuous position and a single `TrackLeft`/`TrackStarted` when the restart lands in the next track.
3. **Pre-roll (not in the ADR).** A cold stretcher overshoots up to 4–5 % for about 60 ms after a reset. The reader is seeked `2 × input_latency` frames early (`TrackReader::seek_with_preroll`, new) and the discard grows by `round(preroll/rate)`. Measured over 109 phases the peak is ≤ 0.5011 with pre-roll, against 0.522 without.
4. **Multi-callback fade-out.** The fade-out spans as many callbacks as it needs, so a device buffer shorter than 5 ms still gets the full ramp. It is shorter only if fewer than 5 ms are queued. A zero-length callback (the tests' empty acknowledgement) cuts instead of fading.
5. **Throttle:** 125 ms, from flush start, using std `Instant`. Changes in between wait, the latest value wins, and the decoder's blocking waits are capped by the deadline. Direct adoption with no flush happens when nothing is audible (nothing loaded, or everything already played).

proposals:
- P: Amend ADR 0006 | why: record the five deviations above, and replace the "step equals steady sine" claim with "+0–2 % stretcher ripple" | size: S | evidence: docs/adr/0006-time-stretch.md decisions 4–6 and the Consequences section
- P: Derive `RateSession` in the controller | why: per play, snapshot `rate_stats()` at `PlayStarted` and at the leave, then emit `time_weighted_rate_since` | size: S/M | evidence: controller/mod.rs `rate_stats()`, engine.rs `RateStats::time_weighted_rate_since`
- P: Expose the rate in `PlaybackStatus` (and a speed control in the UI ticket) | why: the UI should show the engine's applied rate, not its own guess | size: S | evidence: controller/types.rs `PlaybackStatus` has no rate field
- P: Listening pass on the restart dip with pre-roll | why: pre-roll adds about 120 ms of extra stretch work per restart, a few ms of CPU. A real-device measurement of set_rate-to-audio latency (ADR says 10–20 ms) is not in this ticket, and the exclusive-mode reopen is T-0057 | size: S | evidence: decoder.rs `restart_audible`, `StretchStage::preroll_frames`

questions for lead:
- The ticket scope excludes docs/. Do you want the doc updates above done in a follow-up commit by me, or yourself?
- `tests/rate.rs` uses real sleeps of 130 ms, only to clear the throttle interval (a lower bound), so each rate step costs 0.13 s. If that is too slow for the suite, an injectable throttle clock would be a small follow-up.
- Not mine: python.exe PID 60496 (started 2 Nov 10:04, running `t.py cmd C:/ cmake-build.cmd configure`) was already running and I left it. I started nothing that is still running.
