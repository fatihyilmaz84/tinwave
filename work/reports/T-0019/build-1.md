HANDOFF T-0019 (round 1)
status: done
branch: t/T-0019-resampler-and-dither   commit: b89cb0b
summary: Tracks whose rate differs from the stream rate are resampled on the decoder thread with rubato `Fft`. Same-rate tracks share one resampler with no reset at the boundary; a rate change flushes the old resampler's tail and starts a new one; a seek, load or device switch resets it. The `Pump` converts float to integer with TPDF dither only for `I16` and `I24In32`; `F32` and `I32` untouched; samples already on the integer grid pass through, so unity-gain 16/24-bit material stays bit-exact. The audio callback is unchanged. A rate change shows a short filter transient (deviations).

acceptance criteria:
- [x] 44.1 kHz pair on a 48 kHz endpoint gapless (≤ 1 frame) — tests/resampling.rs `a_44k_pair_on_a_48k_stream_is_gapless_within_one_frame`: lag 0 at the boundary, 3,000 frames before and after (±4 frame search), waveform error < 1e-3 (sweep 0.1 peak), exact length; `TrackLeft` frames `round(split*48000/44100)` and the remainder, to the frame. `the_boundary_between_same_rate_tracks_has_no_filter_restart` measures ±200 frames at the join.
- [x] Rate change without glitch or callback allocation — `rate_changes_between_tracks_play_without_a_glitch` (48 k → 44.1 k → 96 k): lag 0 at both joins, exact lengths and `TrackLeft` frames, no `Underrun`; error < 1e-3 except within ±100 frames of a join (< 0.05). tests/rt_alloc.rs `render_never_allocates_across_rate_changes_and_resampled_seeks`: 0 callback allocations across 48 → 44.1 → 96 → 44.1 → 48, a seek and a device switch in resampled tracks, and a queue edit.
- [x] Dither only for integer outputs — convert.rs: `f32_and_i32_are_never_dithered` (bytes equal plain conversion; generator state not advanced), `samples_already_on_the_grid_are_left_alone`, `off_grid_samples_get_triangular_dither_that_keeps_the_mean` (P(-1,0,1) = .02/.66/.32, mean .3), `dither_differs_from_plain_rounding_by_at_most_one_step`, `the_same_seed_gives_the_same_bytes`, `dithered_conversion_still_saturates_and_maps_nan_to_silence`. Through the real `Pump`: output_fake `a_volume_below_one_is_dithered_only_on_the_integer_outputs_that_need_it`. Existing bit-exact I16/I24In32 gapless and seek tests pass with dither on.
- [x] fmt, clippy -D warnings, tests, deny.

checks: fmt pass, clippy pass, tests 595 passed / 0 failed (6 skipped; playback crate 218), deny pass. Second `cargo build -p tinwave-playback` compiled nothing.

Revert tests (applied, failing, restored): new resampler per track (3 resampling tests incl. the no-filter-restart test); no seek-end clamp (`a_seek_to_the_last_frame_of_a_downsampled_track_is_not_out_of_range`); Pump plain rounding (output_fake dither test); dither always on incl. exact samples (convert + output_fake gapless/seek); dither on F32 (`f32_and_i32_are_never_dithered`); no tail flush at end of queue (6 tests); `reset_stream` keeps the resampler (device-change, seek); no flush on rate change (`rate_changes_between_tracks…`); delay not trimmed (2 resample unit tests); odd-FFT-block fix removed (192 k → 44.1 k half-frame phase error); `position()` ignores fed frames (3 tests); unconditional allocation in `Renderer::render` (both rt_alloc tests).

files changed: Cargo.toml (`rubato = "5"` in workspace.dependencies), Cargo.lock; crates/tinwave-playback/Cargo.toml (1 line), src/lib.rs (`mod resample;`, export `convert_dithered` and `Dither`, doc), src/resample.rs (new), src/decoder.rs, src/reader.rs (`length()`), src/engine.rs, src/error.rs, src/output/mod.rs (doc only), src/output/convert.rs (dither + tests), src/output/pump.rs; tests/resampling.rs (new), tests/rt_alloc.rs, tests/output_fake.rs, tests/engine.rs, tests/common/mod.rs.
signals added/changed: none
ui: none
docs updated: rustdoc/module docs only. ADR 0005's not-verified list and docs/rules/architecture.md ("tracks must have the stream rate") are the lead's.

deviations from ticket:
- Rate-change transient: where a resampler starts/stops against silence (rate change, first track after a seek) the FFT filter rings ~80 frames (< 2 ms). Steady-state error ~1e-7 of the 0.1 amplitude; within ±100 frames of a join peaks 5e-3 (48 → 44.1 k) and 2.5e-2 (44.1 → 96 k). Tests assert < 1e-3 away from joins, < 0.05 near them, lag 0 at joins. Real tracks start/end near silence; no reflection/priming added (proposal).
- Continuity lost when a track is appended after the decoder already read the last queued track (the queue-end flush emitted the tail; the appended track gets a fresh resampler via revive). Tracks queued when the decoder reaches the boundary, incl. `replace_after` edits, are continuous.
- Frame units: seek targets, start frames, marks, positions are stream-rate frames (as the controller assumed); `GaplessInfo` stays in file frames. A seek to a stream frame whose file frame isn't whole lands within half a file frame (~0.5 stream frame shift); seek tests use whole-number targets (multiples of 160 at 44.1 kHz).
- An unconvertible rate (zero, or a pair with a tiny common divisor e.g. 44100 → 48001) still fails the track with `SampleRateMismatch`; engine.rs `unplayable_tracks_are_reported_and_skipped` uses 48,001 Hz now.
- `convert_into` stays plain rounding; `convert_dithered` and `Dither` are new public API; I32 not dithered (f32 has 24 significant bits).
- Not done (out of scope): the output stream rate is fixed per engine; exclusive-mode reopen at the file rate (ADR 0005 decision 5) is an engine/controller feature.

CPU (P-0055), release, Ryzen 7 7800X3D, 60 s through `StreamResampler` in 4096-frame pushes: 44.1 → 48 k 0.062 % of a core stereo (1,601x RT), 0.030 % mono; 48 → 44.1 k 0.073 % / 0.024 %; 88.2 → 48 k 0.105 % / 0.040 %; 96 → 48 k 0.067 % / 0.033 % (1,492x); 192 → 48 k 0.136 % / 0.057 % (734x); 192 → 44.1 k 0.147 % / 0.067 %. Mono files are duplicated to stereo before the resampler (stereo cost; not worth a channel mask). Stretch is ~1 % at 44.1 kHz, so the resampler is a minor part of that budget.

proposals:
- P: Sub-frame-accurate seeks into resampled tracks (rubato `Async` first phase or a 1-frame warm-up) | size: S
- P: Continue the resampler across a late append (hold the flush until the ring would underrun, or re-prime with the last input frames) | why: ~80-frame transient on gapless live albums when appended late | size: M
- P: Prime a new resampler with an odd reflection of its first chunk (and the flush with a reflection of the tail) | why: halves the join error on cut-through material; needs a listening test | size: S
- P: Update ADR 0005 "Not verified" and docs/rules/architecture.md with the decoder-thread resampler and the dither policy | size: S
- P: `ManualOutput` with a different device rate, and exclusive-mode reopen at the file rate | why: ADR 0005 decision 5 isn't implemented anywhere (engine fixes the stream rate) | size: M

questions for lead:
- Is < 2 ms of filter transient at a rate change acceptable for v1, or do the reflection priming now?
- python3.exe processes aren't the builder's; nothing started is still running.

---
Lead (2026-11-03):
- < 2 ms transient at a rate change: accepted for v1 (real tracks start and end near silence); priming parked (P-0222)
  for the PO's listening pass.
- P-0220 (sub-frame seeks) parked. P-0221 (late-append continuity) accepted → T-0056 (gapless live albums are a spec
  promise). P-0223 (ADR 0005 / architecture.md text) → lead applies on merge. P-0224 (exclusive reopen at file rate,
  ADR 0005 decision 5) accepted → T-0057.
