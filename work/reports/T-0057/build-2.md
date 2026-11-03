HANDOFF T-0057 (round 2)
status: done
branch: t/T-0057-exclusive-reopen-at-file-rate   commit: f743811 (main ea438b3 merged first; only docs, work and tinwave-db came in)
summary: Round 2 closes the blocking flaky test and the three reviewer-recommended items. The 10 ms fast poll is now bounded, a full command queue delays the Resume instead of losing the output, and `switch_device` during a boundary park has a regression test. The earlier work is unchanged: exclusive mode reopens the output at each file's rate, a refused rate falls back to the engine rate plus resampler, and an automatic fallback emits no `ExclusiveModeToggled` (your ruling).

**Round 2 changes**
1. **Blocking: flaky `flushes >= 1`.** In `a_speed_change_during_the_second_file_restarts_inside_its_own_stream`, after `set_rate` the test now spins (bounded) on empty callbacks plus `poll()` until `rate_stats().flushes >= 1`, then asserts it and plays on.
   - An empty callback cuts the fade without taking audio, so the flush happens while the second file is still being played, whatever the thread scheduling.
   - Stress at 40 copies: see the Stress section below.
2. **Bounded fast poll.**
   - `Engine::reopen_pending()` is false for a drain-waiting (boundary) park while paused. A park with nothing to drain stays pending.
   - New `Engine::poll_soon()` and `Controller::poll_soon()`: stream open, output not reported lost, and `command_pending() || reopen_pending()`. `controller/runtime.rs` uses `poll_soon()`, so there is no fast poll after `OutputLost` or a failed reopen.
   - Tests: `engine::tests::a_boundary_waiting_for_playback_is_not_pending_while_paused` and `a_lost_output_is_not_polled_quickly`. The second covers both the lost stream and the failed reopen (stream none).
3. **Resume on a full queue.**
   - `service_reopen` now stores the new stream first, then calls `deliver_resume`, which uses `try_send`. On Full it keeps the command and its `StreamInfo` in `unsent_resume`, with the stream already running on its empty ring. Every poll retries first, and `OutputReopened` is emitted only once it is delivered.
   - `send()` returns Busy while a resume is stashed, so no command can overtake it. `reopen_pending()` stays true meanwhile.
   - Disconnected still gives `OutputLost`.
   - Test: `engine::tests::a_full_command_queue_delays_the_resume_instead_of_losing_the_output`. It swaps `tx` for a full 1-slot channel and checks: new stream open, no event yet, no second open on the next poll, `append` refused while the queue has room again, then the resume delivered and one `OutputReopened`, once.
4. **Regression test for `switch_device` during a boundary park:** `exclusive_reopen::a_device_switch_while_the_decoder_waits_at_a_boundary_resumes_and_reopens_on_the_new_device`.
   - Opens are [(None,48000), (None,44100), (dac,44100), (dac,96000)].
   - Audio is A's tail from the heard frame plus B, bit-exact.
   - TrackLeft(1) is within 1 frame of 24000 and TrackLeft(2) is 48000, both Finished, and the events are paired.

**Not touched, per your instruction:** `replace_after` during a boundary park (new ticket) and the mapper fallback branch erasing a deferred listener toggle (T-0067).

**Mutation checks for items 2 and 3 (each reverted; all killed):**
- Paused park still pending: killed by `a_boundary_waiting_for_playback...`.
- `poll_soon` ignoring a lost output: killed by `a_lost_output_is_not_polled_quickly`.
- Full queue treated as `OutputLost` (old behaviour): killed by `a_full_command_queue_delays_the_resume...`.
- `send()` guard removed: first run survived, because the full queue refused the command anyway. I made the test check refusal with room in the queue; the mutant is then killed by the same test.

**Acceptance criteria (unchanged, still hold)**
- [x] Exclusive 44.1 k to 96 k reopens at the boundary, both tracks bit-exact, no callback allocation: `exclusive_reopen::a_rate_change_reopens_the_output_at_the_file_rate_and_both_tracks_are_bit_exact`, `a_24_bit_pair_reaches_the_device_as_the_files_integers`, `rt_alloc::render_never_allocates_across_exclusive_reopens`.
- [x] Device refuses the rate: falls back to engine rate plus resampler, reported once: `a_refused_rate_plays_resampled_at_the_engine_rate_and_is_reported_once`, `a_device_that_refuses_exclusive_altogether_falls_back_to_shared_once`, `a_device_switch_keeps_the_rate_of_the_playing_file_or_falls_back`.
- [x] Signals per the catalog, no new kinds, with your ruling: `controller_exclusive` (3 tests) and `controller_signals`. A rate reopen emits no signal. A refusal emits `OutputReopened{fallback}` and a warn log, no signal. Only the listener or setting turning exclusive off or on emits `ExclusiveModeToggled`, once each.
- [x] Same-rate boundaries never reopen and stay gapless: `same_rate_tracks_in_exclusive_mode_never_reopen_and_stay_gapless`.
- [x] Composes with T-0019 and T-0027 (positions, marks, `RateStats`, stretched rate across the reopen): the stretch tests, `positions_and_marks_continue_across_the_reopen`, `the_position_stays_on_the_old_track_until_the_new_stream_is_heard`.

**checks:** fmt pass, clippy `--workspace --all-targets -D warnings` pass, tests 767 passed / 0 failed (6 skipped: ignored real-device tests, not run), deny pass (advisories, bans, licenses, sources ok).

**Stress (40 copies):** run in parallel, 3 rounds each, with the other binaries running at the same time.
- exclusive_reopen (the previously flaky binary): 0 of 120 failed runs.
- controller_exclusive: 0 of 90.
- lib unit tests (engine, decoder, renderer, shared): 0 of 90.
- rt_alloc: 0 of 60 (a separate pass of 0 of 90).
- controller_thread, the pre-existing test file: 6 failed runs of `dropping_the_handle_ends_the_session` and 1 of `queue_edits_go_through_the_handle...`. This only happens when 40 copies of every binary run at once. 40 copies of controller_thread alone gave 0 of 120, on the branch and on a baseline build of main 7a6cf5d.
- Under a comparable load (40 copies of exclusive_reopen and 30 of rt_alloc running alongside), the baseline main 7a6cf5d controller_thread also failed 2 of 120. So this is pre-existing load flakiness in controller_thread (shared-mode, signal timing), not caused by this ticket. I left it alone and suggest a separate hardening ticket.
- The baseline scratch worktree and its target dir were removed.

**files changed this round (all in crates/tinwave-playback/):** src/engine.rs, src/controller/mod.rs, src/controller/runtime.rs, tests/exclusive_reopen.rs. Everything else from round 1 is unchanged.

**signals added/changed:** none (no new kinds, no bytes/day).

**ui:** none

**docs updated (in crate only):** the `reopen_pending`, `poll_soon` and `deliver_resume` docs and the runtime.rs module doc.

**Docs for the lead on merge (unchanged from round 1; `docs/` untouched):**
- ADR 0005 decision 5 (implemented, fakes only) and "Not verified".
- signal-catalog `ExclusiveModeToggled` row (listener/setting changes only; automatic fallbacks don't emit; a rate reopen is not a toggle).
- docs/rules/architecture.md: tinwave-playback `wants_exclusive` and the reopen path.
- PO-RUNBOOK: manual exclusive check on a real DAC (44.1 → 96 kHz album).

**deviations from ticket:** none in scope (as in round 1: `AudioOutput::wants_exclusive`, the controller fast poll with `poll_soon`/`reopen_pending`, `ManualOutput::set_exclusive` / `set_device_rates`; all inside crates/tinwave-playback/).

**Known limits (unchanged):**
- The real gap is unmeasured: old device buffer + exclusive open (about 42 ms in the spike) + first block + up to 10 ms of poll.
- A real endpoint's refusal of 96 kHz is untested.
- A queue edit in the few-ms window right after a load or seek has no dedicated test.
- A Resume for a request that is no longer pending is logged and ignored (unreachable by construction).

**proposals:**
- P: Harden controller_thread tests under heavy parallel load | why: `dropping_the_handle_ends_the_session` and `queue_edits_go_through_the_handle...` fail about 2 to 5 % of runs at 40+ concurrent processes, on baseline main as well | size: S | evidence: tests/controller_thread.rs:297 and :343 (timing assumptions about how far playback got).
- P: Real-device exclusive reopen check | why: nothing here has touched an endpoint | size: S | evidence: tests/device.rs (ignored, silent-clip pattern).
- P: Skip the wasted shared open on a refused rate | why: WasapiOutput opens a shared stream at the refused rate before the engine closes it | size: S | evidence: src/output/windows.rs `open_device`.
- P: Make `common::Harness::settle` poll | why: it spins forever in exclusive mode when a load needs a reopen | size: S | evidence: tests/common/mod.rs.

**questions for lead:** none.
