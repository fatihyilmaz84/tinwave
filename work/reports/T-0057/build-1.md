HANDOFF T-0057 (round 1)
status: done
branch: t/T-0057-exclusive-reopen-at-file-rate   commit: 65c6f1f
summary: In exclusive mode the output is reopened at each file's sample rate at the track boundary, with no resampler, stretcher or dither on the 1.0 path. A device that refuses the rate gets the engine's own rate plus the decoder resampler, reported once in `StreamInfo.fallback`. All of it runs on ManualOutput and the fake pump device; no real device was opened, and the gap on real hardware is unmeasured.

How it works (control flow):
- The decoder cannot open streams. In exclusive mode, when the next track's file rate differs from the stream rate, it parks (`Shared::park`) and publishes the rate it needs. At a boundary it first pushes the stretcher and resampler tails.
- `Engine::poll` answers (`service_reopen`) when the old stream has played its last frame and its device buffer has emptied. The renderer counts silence in a new `idle_frames` counter, which the engine compares with the reported latency. After a load or seek there is nothing to wait for.
- The engine closes the old stream and opens a new one via `open_stream`. It then sends `Cmd::Resume`; the decoder keeps the old marks and continues with a Continuation mark.
- Marks now carry `out_rate` and `timeline_rate`. Positions, seek targets and start frames stay in timeline frames at `EngineConfig::sample_rate`, whatever rate the stream runs at.
- `AudioOutput::wants_exclusive()` is new, default `false`. `WasapiOutput` returns its control flag and `ManualOutput` returns its own flag.
- A refused rate shows up as a stream whose mode is not Exclusive. The engine closes it, remembers the rate in `Shared::refused` and opens at the engine rate, which is exclusive if the device allows it and shared otherwise. The decoder then resamples that rate. The refused set resets on `switch_device`.
- `switch_device` now opens the new stream before sending `Reconfigure` (which carries `rate`). In exclusive mode it keeps the playing file's rate.
- The controller thread polls every 10 ms while a command or a reopen is pending, so a reopen does not wait out the 100 ms interval. Without this, a load into another rate would wait up to one full poll. New: `Engine::reopen_pending` and `Controller::reopen_pending`.

acceptance criteria:
- [x] Exclusive, 44.1 k -> 96 k: output reopens at 96 k at the boundary, both tracks bit-exact, no callback allocation — proof:
  - `exclusive_reopen::a_rate_change_reopens_the_output_at_the_file_rate_and_both_tracks_are_bit_exact`: opens [48000, 44100, 96000], concatenated 24-bit audio equals the references exactly.
  - `a_24_bit_pair_reaches_the_device_as_the_files_integers`: through the real Pump onto an I24In32 fake device, the bytes are the file's integers (no dither).
  - `rt_alloc::render_never_allocates_across_exclusive_reopens`: zero allocations across boundary and load reopens.
- [x] Device refuses the rate: shared (or exclusive at the engine rate) plus resampler, reported once — proof:
  - `a_refused_rate_plays_resampled_at_the_engine_rate_and_is_reported_once`: one OutputReopened carries the fallback; a later 96 kHz track does not retry; 44.1 kHz tracks stay bit-exact.
  - `a_device_that_refuses_exclusive_altogether_falls_back_to_shared_once`: the mode goes to Shared once and the third track makes no new open.
  - `a_device_switch_keeps_the_rate_of_the_playing_file_or_falls_back`.
- [x] Signals per catalog, no new kinds — proof: `controller_exclusive.rs` (two tests).
  - A reopen at another rate emits no signal.
  - Losing exclusive mode emits exactly one `ExclusiveModeToggled{on:false}`, and the queue still completes 3/3.
  - Events: a rate reopen (exclusive to exclusive) is one `OutputReopened`, mapper silent. A refusal that keeps exclusive at the engine rate is one `OutputReopened` with `fallback` (mapper logs warn, no signal). A refusal that ends exclusive is one `OutputReopened{mode Shared, fallback}` and the mapper emits one toggle off. `OutputLost` only if no stream can be opened. Same-rate boundaries emit nothing. `OutputOpened` is unchanged.
- [x] Gapless: same-rate boundaries never reopen — proof: `same_rate_tracks_in_exclusive_mode_never_reopen_and_stay_gapless` (3 pieces, `render_exact` with no starvation, bit-exact, one open for the first track only). Existing shared-mode gapless and resampling tests pass unchanged.
- [x] Composes with T-0019/T-0027:
  - `a_rate_change_in_the_playback_speed_composes_with_the_reopen`: rate 1.25 across the reopen, time-weighted rate within 0.2 % of 1.25, TrackLeft frames within 16 of 48,000.
  - `a_speed_change_during_the_second_file_restarts_inside_its_own_stream`: a rate flush in the 96 kHz stream does not reopen.
  - `positions_and_marks_continue_across_the_reopen`.
  - `the_position_stays_on_the_old_track_until_the_new_stream_is_heard`.
- [x] fmt, clippy, tests, deny pass (below).

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`), tests 747 passed / 0 failed (6 skipped = ignored real-device tests, not run), deny pass (advisories, bans, licenses, sources ok). A second `cargo build -p tinwave-playback` compiled nothing.

Mutation checks (each reverted afterwards; nothing survives):
- No device-buffer drain wait: killed by `a_rate_change_reopens...bit_exact`.
- No generation guard: killed by `engine::tests::a_reopen_request_is_served_only_for_the_latest_command...`.
- Refused set ignored: killed by `decoder::tests::a_refused_rate_is_converted...`.
- `out_rate` ignored in marks: 3 tests.
- Fade frames not updated per stream: killed by the engine test.
- Commands no longer cancel a park: killed by `a_load_while_the_decoder_waits_for_a_stream_cancels_the_wait`.
- Idle counter removed: killed by `renderer::tests::silence_after_the_last_audio...`.
- Buffering not set at park: killed by 3 exclusive_reopen tests (underrun at the boundary).
- Fast poll removed: killed by `controller_thread::a_command_that_needs_another_stream_is_not_left_waiting_for_a_slow_poll`.
- Marks dropped at the boundary: killed by `the_position_stays_on_the_old_track_until_the_new_stream_is_heard`.

Stress: 8 parallel copies x 6 rounds of the exclusive_reopen, controller_exclusive, controller_thread and rt_alloc binaries plus the engine/decoder/renderer/shared unit tests: 0 failures. Stress found two flaky tests, both fixed in the tests (not the engine). Tests wait on conditions (buffered frames, `reopen_pending`, `command_pending`), never on fixed sleeps. Both fixes assert only what is deterministic.
- The fake-device test stepped the device without waiting for decoded blocks, which put silence mid-recording.
- The speed-change test counted frames rendered after `set_rate`, which depends on how fast the decoder applies the flush.
- Underruns at the very end of a queue (the last callback racing the decoder's end-of-stream flag) are not judged; the tests count only underruns before the last track started.

files changed (all in crates/tinwave-playback/):
- src/: decoder.rs, engine.rs, shared.rs, renderer.rs, event.rs, lib.rs, output/mod.rs, output/windows.rs, controller/mod.rs, controller/runtime.rs
- tests/: exclusive_reopen.rs (14 tests, new), controller_exclusive.rs (new), common/mod.rs (Harness::exclusive, settle_polling, fake device exclusive support), controller_thread.rs, rt_alloc.rs
- Unit tests were added inside engine.rs, decoder.rs, renderer.rs and shared.rs.

signals added/changed: none (no new kinds, no new bytes/day). Mapping used: OutputReopened -> ExclusiveModeToggled only when the mode differs (existing T-0024/T-0034 rule).

ui: none

docs updated (in-crate only, `docs/` untouched):
- Module docs in decoder.rs (new "Exclusive mode: the stream follows the file" section) and lib.rs.
- The `OutputReopened` doc in event.rs and the controller signal table row in controller/mod.rs.
- Docs on `EngineConfig::sample_rate`, `OpenRequest::sample_rate`, `RateStats::wall_frames`, `command_pending`, `WasapiOutput`.

Docs the lead should change on merge:
- ADR 0005 decision 5: mark exclusive reopen implemented (T-0057), verified on fakes only.
- ADR 0005 "Not verified": replace the "Exclusive mode still plays every track at the stream rate; reopening at the file rate is T-0057" sentence. New text: implemented and tested on ManualOutput and the fake pump device; not verified on a real endpoint, so the real gap (old device buffer plus exclusive open, about 42 ms in the spike, plus up to 10 ms of poll) and a real endpoint's refusal of 96 kHz remain open. The spike measured only the shared-mode 25 ms device switch.
- docs/guides/signal-catalog.md, `ExclusiveModeToggled` row: add that a reopen at another sample rate is not a toggle, and an automatic fallback that ends exclusive mode is one `on:false`, not repeated.
- docs/rules/architecture.md: tinwave-playback description should mention `AudioOutput::wants_exclusive` and the reopen path.
- docs/guides/desktop-dev-setup.md / PO-RUNBOOK: manual exclusive check on a real DAC (44.1 -> 96 kHz album, listen for the gap).

deviations from ticket: none in scope. Beyond the two bullets:
- `AudioOutput::wants_exclusive` (trait, default false).
- Controller fast poll (10 ms) while a command or a reopen is pending.
- `Engine::reopen_pending` and `Controller::reopen_pending`.
- `ManualOutput::set_exclusive` and `set_device_rates`.
- All inside crates/tinwave-playback/.

Gap, honestly:
- A rate-changed reopen is not gapless (no way around it). What I measured is on the fake: the old stream is held until its last frame has played and the device buffer has emptied, so no audio is cut. The test asserts at least the 480 latency frames of silence before the reopen.
- Real gap = device buffer (about 4 ms exclusive, 22 ms shared in the spike) + the exclusive open + the decoder opening the next file and filling the first block + up to one poll (10 ms while a reopen is pending) — not measured on a device.
- Same-rate boundaries never reopen.
- A refusal costs two extra opens: the failed attempt, then the retry at the engine rate. The WASAPI refusal path opens a shared stream at the refused rate, which the engine then closes. This happens once per refused rate per device.

Known limits and untested paths:
- A real WASAPI exclusive refusal and real exclusive open timing are unverified, and I did not add an ignored device test because I could not run it.
- A queue edit that arrives in the few-ms window while the decoder is parked right after a load or seek is handled (`head_is_cur`) but has no dedicated test, because the window cannot be forced deterministically.
- A Resume answering a request that is no longer pending is logged as an error and ignored. It is unreachable by construction (the engine answers only the request of its latest command) and has no test.
- `Engine::start` opens at the engine rate only. If an exclusive device refuses that rate, `OutputOpened` carries the fallback and the stream stays shared for the session (follow off).
- In exclusive mode `RateStats.wall_frames` counts frames of whichever rate played them, now documented. The time-weighted rate (ratio) is unaffected.

proposals:
- P: Real-device exclusive reopen check | why: nothing here has touched an endpoint, so the gap and a real 96 kHz refusal are unmeasured | size: S | evidence: crates/tinwave-playback/tests/device.rs (ignored, silent-clip pattern) could gain one test that plays a 44.1 kHz then 96 kHz silent WAV pair exclusively and records the reopen time, for the PO to run.
- P: Skip the wasted shared open on a refused rate | why: WasapiOutput opens a shared stream at the refused rate before the engine closes it (an extra 25 ms or more) | size: S | evidence: crates/tinwave-playback/src/output/windows.rs `open_device` (the resolve_mode fallback); an OpenRequest flag "exclusive only, fail instead of falling back" would let the engine retry at its own rate in one open.
- P: Let `Engine` wake its host when a reopen is pending | why: the controller now polls faster while pending, but a host that owns its own poll loop still waits for its next tick | size: S | evidence: `Engine::reopen_pending` in engine.rs; a callback or channel would remove the polling.
- P: Extend common::Harness::settle to poll | why: `settle()` spins forever in exclusive mode when a load needs a reopen (tests must use `settle_polling`) | size: S | evidence: tests/common/mod.rs `settle` vs `settle_polling`.

questions for lead:
- None. One thing to confirm if it matters: an automatic refusal that ends exclusive mode emits `ExclusiveModeToggled{on:false}` by the existing mapper rule, so a refusal looks like the user turning exclusive off in the signal stream. If you would rather keep automatic fallbacks out of the preference signals, that is a one-line mapper change plus a catalog note.
