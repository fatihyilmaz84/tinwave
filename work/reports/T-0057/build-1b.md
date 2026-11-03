HANDOFF T-0057 (round 1, updated for the lead ruling)
status: done
branch: t/T-0057-exclusive-reopen-at-file-rate   commit: e3530de
summary: In exclusive mode the output is reopened at each file's sample rate at the track boundary, with no resampler, stretcher or dither on the 1.0 path. A device that refuses the rate gets the engine's own rate plus the decoder resampler, reported once in `StreamInfo.fallback`. Per your ruling, an automatic fallback now emits no `ExclusiveModeToggled`; only listener or setting changes do. Everything was verified on ManualOutput and the fake pump device; no real device was opened, so the real gap is unmeasured.

**The ruling, as implemented**
- `mapper.rs::output_reopened`: an `OutputReopened` whose `info.fallback` is set only logs at warn.
- It does set `mode` and `reported_mode` to the stream's mode, so the fallback mode is the new starting state for the next listener change. No signal is emitted and no deferred toggle is left behind.
- A reopen with no fallback keeps the old rule: a mode change emits `ExclusiveModeToggled` (listener or setting turns exclusive off or on, deferred and coalesced as before).
- The controller signal table row (controller/mod.rs) and the `OutputReopened` doc (event.rs) say this.
- `controller_exclusive.rs::losing_exclusive_mode_to_a_refusing_device_is_no_preference_signal_and_the_queue_plays_on`: refusal produces zero toggles, all 3 tracks complete, and the opens are [48000, 44100, 96000, 48000].
- `controller_exclusive.rs::the_listener_turning_exclusive_mode_off_and_on_is_one_toggle_each`: `set_exclusive(false)` plus `switch_device` gives exactly one `on:false`; `set_exclusive(true)` plus `switch_device` gives exactly one `on:true`.
- `controller_signals.rs::exclusive_mode_toggled_follows_the_stream_info_of_a_reopened_output`: its last stage encoded the old behaviour (fallback gave a toggle off). It now asserts an automatic fallback emits nothing.
- Mutation check: I removed the fallback branch (re-enabling the fallback toggle). The refusal test and `exclusive_mode_toggled_follows_the_stream_info...` failed. The change was reverted and then re-applied; the commit has the fix.
- Known consequence: if the listener turns exclusive off while the stream is already in the fallback Shared mode, there is no mode change and so no signal. The mapper cannot see the control flag.

**How it works (unchanged from the earlier hand-back)**
- The decoder cannot open streams. In exclusive mode, when the next track's rate differs from the stream's, it parks: it pushes the stretcher and resampler tails, then publishes the rate it needs.
- `Engine::poll` answers (`service_reopen`) once the old stream has played its last frame and its device buffer has emptied, measured with the renderer's `idle_frames` counter against the reported latency. After a load or seek it answers at once.
- `Cmd::Resume` carries the new ring. Older marks are kept and the next track gets a Continuation mark. Marks carry `out_rate` and `timeline_rate`, so positions, seeks and start frames stay in timeline frames at `EngineConfig::sample_rate`.
- `AudioOutput::wants_exclusive()` is new (default false; `WasapiOutput` and `ManualOutput` implement it).
- A refused rate shows up as a stream whose mode is not Exclusive. The engine closes it, remembers the rate in `Shared::refused` (reset on `switch_device`) and opens at the engine rate, which stays exclusive if the device allows it. The decoder then resamples that rate.
- `switch_device` now opens the new stream before sending `Reconfigure` (which carries `rate`), and keeps the playing file's rate in exclusive mode.
- The controller thread polls every 10 ms while a command or a reopen is pending (`Engine::reopen_pending`, `Controller::reopen_pending`).

**Acceptance criteria**
- [x] Exclusive 44.1 k to 96 k reopens at 96 k at the boundary, both tracks bit-exact, no callback allocation.
  - `exclusive_reopen::a_rate_change_reopens_the_output_at_the_file_rate_and_both_tracks_are_bit_exact`: opens are [48000, 44100, 96000] and the audio equals the 24-bit references exactly.
  - `a_24_bit_pair_reaches_the_device_as_the_files_integers`: through the real Pump to an I24In32 fake device, no dither.
  - `rt_alloc::render_never_allocates_across_exclusive_reopens`.
- [x] Device refuses the rate: falls back to the engine rate plus resampler, reported once.
  - `a_refused_rate_plays_resampled_at_the_engine_rate_and_is_reported_once`.
  - `a_device_that_refuses_exclusive_altogether_falls_back_to_shared_once`.
  - `a_device_switch_keeps_the_rate_of_the_playing_file_or_falls_back`.
- [x] Signals per the catalog (no new kinds), with the ruling applied. Events:
  - A rate reopen (exclusive to exclusive) is one `OutputReopened`, no signal.
  - A refusal that keeps exclusive at the engine rate is one `OutputReopened` with `fallback`, a warn log, no signal.
  - A refusal that ends exclusive is one `OutputReopened{mode Shared, fallback}`, a warn log, no signal.
  - Listener turning exclusive off or on is one `ExclusiveModeToggled`.
  - `OutputLost` only if no stream can be opened.
  - `OutputOpened` is unchanged.
  - Proof: `controller_exclusive.rs` (3 tests) and `controller_signals.rs`.
- [x] Same-rate boundaries in exclusive mode never reopen and stay gapless: `same_rate_tracks_in_exclusive_mode_never_reopen_and_stay_gapless`. Existing shared-mode gapless and resampling tests pass unchanged.
- [x] Composes with T-0019 and T-0027:
  - `a_rate_change_in_the_playback_speed_composes_with_the_reopen`.
  - `a_speed_change_during_the_second_file_restarts_inside_its_own_stream`.
  - `positions_and_marks_continue_across_the_reopen`.
  - `the_position_stays_on_the_old_track_until_the_new_stream_is_heard`.
  - `a_load_while_the_decoder_waits_for_a_stream_cancels_the_wait`.
- [x] fmt, clippy, tests, deny pass.

**checks:** fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 748 passed / 0 failed (6 skipped: ignored real-device tests, not run), deny pass (advisories, bans, licenses, sources ok). A second `cargo build -p tinwave-playback` compiled nothing.

**Mutation checks (all reverted; none survive):**
- No device-buffer drain wait: killed.
- No generation guard: killed.
- Refused set ignored: killed.
- `out_rate` ignored: 3 tests.
- Fade frames not updated per stream: killed.
- Commands no longer cancel a park: killed.
- Idle counter removed: killed.
- Buffering not set at park: 3 tests.
- Fast poll removed: killed.
- Marks dropped at the boundary: killed.
- Fallback toggle re-enabled: 2 tests (this round).

**Stress:** 8 parallel copies × 6 rounds of the exclusive_reopen, controller_exclusive, controller_thread and rt_alloc binaries plus the engine, decoder, renderer and shared unit tests gave 0 failures. That run was before the round-1 ruling edits, which touch only the mapper, docs and two controller tests, not the timing-sensitive engine tests. Tests wait on conditions, not sleeps. The earlier stress run found two flaky tests; I fixed the tests, not the engine. Underruns at the very end of a queue are not judged, because the last callback races the decoder's end-of-stream flag.

**files changed (all in crates/tinwave-playback/):**
- src/: decoder.rs, engine.rs, shared.rs, renderer.rs, event.rs, lib.rs, output/mod.rs, output/windows.rs, controller/mod.rs, controller/runtime.rs, controller/mapper.rs
- tests/: exclusive_reopen.rs (14 tests, new), controller_exclusive.rs (3 tests, new), common/mod.rs, controller_thread.rs, controller_signals.rs, rt_alloc.rs
- Unit tests were added inside engine.rs, decoder.rs, renderer.rs and shared.rs.

**signals added/changed:** no new kinds, no new bytes/day. `ExclusiveModeToggled` is emitted for listener or setting changes only; an automatic fallback emits nothing (this round's change).

**ui:** none

**docs updated (in crate only):** module docs in decoder.rs and lib.rs, the `OutputReopened` doc in event.rs, the controller/mod.rs signal table row, and docs on `EngineConfig::sample_rate`, `OpenRequest::sample_rate`, `RateStats::wall_frames`, `command_pending` and `WasapiOutput`.

**Docs for the lead to change on merge (`docs/` untouched):**
- ADR 0005 decision 5: mark exclusive reopen implemented (T-0057), verified on fakes only.
- ADR 0005 "Not verified": replace "Exclusive mode still plays every track at the stream rate; reopening at the file rate is T-0057". New text: implemented and tested on ManualOutput and the fake pump device. Not verified on a real endpoint, so the real gap and a real 96 kHz refusal are open.
- signal-catalog `ExclusiveModeToggled` row: your planned "listener/setting changes only; automatic fallbacks don't emit", plus that a reopen at another rate is not a toggle.
- docs/rules/architecture.md: tinwave-playback description should mention `AudioOutput::wants_exclusive` and the reopen path.
- PO-RUNBOOK: manual exclusive check on a real DAC (44.1 → 96 kHz album).

**deviations from ticket:** none in scope. Beyond the two bullets I added `AudioOutput::wants_exclusive`, the 10 ms controller fast poll with `reopen_pending` on `Engine` and `Controller`, and `ManualOutput::set_exclusive` / `set_device_rates`. All are inside crates/tinwave-playback/.

**Gap, honestly:**
- A rate-changed reopen is not gapless. On the fake, the old stream is held until its last frame has played and the device buffer has emptied, so no audio is cut; the test asserts at least the 480 latency frames of silence before the reopen.
- Real gap = old device buffer (about 4 ms exclusive, 22 ms shared in the spike) + exclusive open (about 42 ms in the spike) + the decoder opening the next file and filling the first block + up to 10 ms of poll. Not measured on a device.
- Same-rate boundaries never reopen.
- A refusal costs two extra opens: the failed attempt (WASAPI opens a shared stream at the refused rate), then the retry at the engine rate. This happens once per refused rate per device.

**Known limits:**
- A queue edit landing in the few-ms window while the decoder is parked right after a load or seek is handled (`head_is_cur`) but has no dedicated test, because the window cannot be forced deterministically.
- A Resume answering a request that is no longer pending is logged as an error and ignored. It is unreachable by construction and has no test.
- `Engine::start` opens at the engine rate only. If an exclusive device refuses that rate, `OutputOpened` carries the fallback and the stream stays shared for the session.
- In exclusive mode `RateStats.wall_frames` counts frames of whichever rate played them (documented); the time-weighted rate is unaffected.

**proposals:**
- P: Real-device exclusive reopen check | why: nothing here has touched an endpoint | size: S | evidence: crates/tinwave-playback/tests/device.rs (ignored, silent-clip pattern) could gain a 44.1 → 96 kHz silent WAV pair test that times the reopen, for the PO to run.
- P: Skip the wasted shared open on a refused rate | why: WasapiOutput opens a shared stream at the refused rate before the engine closes it | size: S | evidence: crates/tinwave-playback/src/output/windows.rs `open_device` (the resolve_mode fallback); an OpenRequest flag "exclusive only, fail instead of falling back" would allow one open.
- P: Let `Engine` wake its host when a reopen is pending | why: the controller now polls faster, but a host with its own poll loop still waits for its next tick | size: S | evidence: `Engine::reopen_pending` in engine.rs.
- P: Make `common::Harness::settle` poll | why: it spins forever in exclusive mode when a load needs a reopen (tests must use `settle_polling`) | size: S | evidence: tests/common/mod.rs.
- P: Persist the listener's exclusive setting in the controller | why: with the ruling, a listener turning exclusive off while already in fallback Shared emits no signal because the mapper only sees stream modes | size: M | evidence: mapper.rs `output_reopened`; the control flag would have to reach the controller.

**questions for lead:** none.
