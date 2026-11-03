REVIEW T-0019 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy -D warnings pass; tests pass on one run (nextest --workspace: 595 passed, 6 skipped) but one new test fails ~1 in 9 under parallel load (blocking); deny pass

The code itself looks correct; the only blocker is one timing-dependent test.

merge with main (T-0026): `git merge-tree --write-tree main t/T-0019-resampler-and-dither` → cfda917, no conflicts. Root Cargo.toml has `cc = "1"` and `rubato = "5"`; the crate has `rubato = { workspace = true }` and the `cc` build-dep; lib.rs has `mod resample;` and `pub mod stretch;`. Merged tree built in a scratch copy with its own target dir (`--locked`): `cargo nextest run -p tinwave-playback` 232 passed, 4 skipped. Scratch deleted.

acceptance criteria:
- [x] 44.1 kHz pair on a 48 kHz endpoint gapless (≤ 1 frame) — `a_44k_pair_on_a_48k_stream_is_gapless_within_one_frame` (lag 0 at the boundary and ±3,000 frames, ±4 search; error < 1e-3; exact length; exact TrackLeft frames); `the_boundary_between_same_rate_tracks_has_no_filter_restart`; unit `one_continuous_resampler_has_no_error_at_a_track_boundary`. Deterministic; 0 failures in 60 stressed runs.
- [x] Rate change without glitch or callback allocation — `rate_changes_between_tracks_play_without_a_glitch` (48 → 44.1 → 96 kHz: lag 0 at both joins, exact TrackLeft frames, no Underrun; < 2 ms transient accepted); `rt_alloc.rs::render_never_allocates_across_rate_changes_and_resampled_seeks` (0 allocations, 30 runs under load). Renderer not in the diff; Pump adds one `u64` of dither state.
- [x] Dither only for integer outputs — convert.rs `f32_and_i32_are_never_dithered`, `samples_already_on_the_grid_are_left_alone` (all 65,536 16-bit values exact), TPDF distribution (P = .02/.66/.32, mean .3), `the_same_seed_gives_the_same_bytes`, saturation/NaN; end to end `output_fake.rs::a_volume_below_one_is_dithered_only_on_the_integer_outputs_that_need_it`. Generator xorshift64* with two disjoint 24-bit fields → triangular (−1, 1) LSB; NaN → 0, ±inf saturates (I16, I24In32).
- [ ] fmt, clippy, tests, deny — suite not reliable (blocking).

blocking findings:
- tests/resampling.rs:263-290 `a_queue_edit_keeps_the_resampler_running_across_the_new_tail` races the decoder: p1 is 20,001 frames at 44.1 kHz (~21,770 stream frames) while the ring holds 48,000 (`RING_SECONDS = 1`), so the decoder can finish p1 and open p3 before `replace_after(QueueItemId(1), …)` arrives → refused as stale (decoder.rs:365-386) → queue stays [p1, p3] (~52,464 frames) → `render_exact(TOTAL - 4_800)` times out (common/mod.rs:225). 0/30 solo; 7/60 in 15 rounds of 12 binaries in parallel ("decoder did not deliver 480 frames in time"); every other binary passed. queue_edits tests avoid it (head CUT = 100,777 > ring). Fix: p1 longer than the ring plus margin (e.g. cut at 60,000 file frames ≈ 65,306 stream frames; p2 = 60,000..75,000; p3 = 75,000..88,200) or the `Gate` opener; assert `QueueEdited` right after the edit so a lost race fails clearly.

design check: n/a

non-blocking findings:
- decoder.rs:502-509 — no test covers the seek-past-end guard for a resampled track: dropping `&& frame <= scale_frames(length, rate, self.sample_rate)` (scratch) makes past-end seeks clamp instead of `SeekFailed`, and all 232 playback tests still pass. Add a past-end seek on a 44.1/96 kHz file next to `seek_past_the_end_is_reported_for_wav_flac_alac_and_mp3`. (Removing `st.resampler = None` in `reset_stream` is caught by 2 tests.)
- rt_alloc.rs:199-205 — the "queue edit while a resampler is running" step probably never applies (playback ~29k frames into item 3 of 33.6k; decoder a second ahead in item 4 → `replace_after(QueueItemId(3), …)` refused); comment false; no `QueueEdited` assertion. Doesn't affect the allocation guarantee.
- decoder.rs:577-585 and :530 — bug-only path where the resampler flush errors: at queue end `st.cur.checked_sub(1)` can name a failed item (`st.last_opened` is right); in `use_rate` a flush error is reported as `TrackFailed` for the new track while the old track's tail is lost and that track isn't marked failed.
- output/pump.rs:107-112 — `Pump::with_dither` public, "only tests need this", unused: remove or use.
- output/convert.rs:108-114 — dither decided per sample: at gains that put only some samples on the grid (e.g. exactly 0.5 on 16-bit into I16) only odd samples get dithered, so the noise floor follows the signal's last bit. Unlikely audible at 16-bit LSB; a design note for the PO listening pass.
- Scope: root `[workspace.dependencies]` and Cargo.lock changes needed for rubato; licences ok (rubato, audioadapter* MIT OR Apache-2.0; realfft, windowfunctions MIT; visibility Zlib/MIT/Apache-2.0; audio-codec-algorithms 0BSD OR Apache-2.0).

Verified with no issue: resampling only on the decoder thread; one resampler continues across same-rate tracks incl. `replace_after` while the head is read; on a rate change the old resampler is flushed into `pending` before the new one starts at `stream_end`; `reset_stream` drops it (seek, load, device switch); at queue end the tail is flushed before `finish_queue` (`take()` prevents a double emit); marks use `resampler.position()`; seek/start/resume frames are stream frames converted to file frames only at `reader.seek`; GaplessInfo stays in file frames; the ADR 0006 seam (`convert_chunk` on file-rate `st.chunk`) is clean for T-0027.

proposals:
- P: Assert `QueueEdited` after every `replace_after` in tests relying on the edit (resampling, rt_alloc) | size: S

notes for lead: once the test fix lands this is an approval; merge with main clean, merged tree passes. P-0223 is the lead's on merge. A `python` process (PID 60496, 2026-11-02) is running; not the reviewer's. Scratch deleted; worktree clean at b89cb0b.

---
Lead (2026-11-03): round 2 = the flaky test (blocking) + QueueEdited asserts (P-0230, merged here) + the rt_alloc edit
step made real + a past-end seek test for a resampled track + correct item attribution on the flush-error paths +
remove `Pump::with_dither`. The per-sample dither note → P-0231, parked for the PO listening pass.
