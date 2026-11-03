HANDOFF T-0019 (round 2)
status: done
branch: t/T-0019-resampler-and-dither   commit: d6d7e74
summary: The blocking race is fixed and the non-blocking items are done. `a_queue_edit_keeps_the_resampler_running_across_the_new_tail` uses a first piece longer than the ring and asserts the edit was applied. The `rt_alloc` edit steps are made real and asserted. A past-end seek on resampled tracks is tested, flush-failure attribution is corrected with a unit test, and the unused `Pump::with_dither` is removed. Source changes only in `decoder.rs` and `pump.rs`; the rest is tests.

acceptance criteria (met in round 1, still hold): gapless 44.1 kHz pair on 48 kHz; rate change without glitch or callback allocation (rt_alloc rewritten, below); dither only for integer outputs; fmt, clippy, tests, deny.

Round-2 fixes:
- Blocking race — the first piece is now 60,000 file frames (~65,300 stream frames), so the decoder can't have opened the next item at the edit (ring 48,000; 4,800 played); pieces p1 = 0..60,000, p2 = 60,000..75,000, p3 = 75,000..88,200. New `Harness::expect_edit_applied(EditId)` (tests/common/mod.rs) waits for that edit's `QueueEdited` and panics with the event list on `QueueEditRejected` (doesn't render). Stress: `resampling`, `rt_alloc`, `failures` binaries, 12 processes in parallel × 20 rounds (720 runs), 0 failures (binaries run directly, not the reviewer's exact harness).
- P-0230 + rt_alloc edit steps — the first rt_alloc test's two `replace_after` calls (probably refused before) now run right after `settle`, before anything plays (ring full with 48,000 of A's 48,777 frames; the decoder can't finish A until the callback takes frames), each followed by `expect_edit_applied`; comment fixed. The rate-change rt_alloc test starts with an 80,000-frame 44.1 kHz track (~87,000 stream frames), edits its tail after 8,820 played frames and asserts it applied, then a device switch and a seek inside the resampled track, then 44.1 → 48 → 44.1 → 96 → 48 kHz and past the end; asserts 0 callback allocations, > 190,000 real frames, and starved callbacks.
- Past-end seek on resampled tracks — `seek_past_the_end_of_a_resampled_track_is_reported` (tests/failures.rs): 44.1 kHz (44,877 frames) and 96 kHz (96,001, odd), targets stream length + 1, + 5,000 and 10,000,000; each `SeekFailed(SeekOutOfRange)` for the seeked item, then the next track starts as `Advance`. Mutation: dropping `&& frame <= scale_frames(...)` fails it.
- Flush-failure attribution — new `flush_failed` in decoder.rs blames `st.last_opened` and sets `item_failed`; both flush-error paths use it (end-of-queue branch of `fill_pending`, and `use_rate`, where the old track is now marked failed). Unit test `decoder::tests::a_failed_flush_blames_the_last_track_that_was_read` (queue of three, `last_opened` = 0, `cur` = 3 → `TrackFailed` for item 1); reverting to `cur - 1` fails it. The error can't be provoked in a test (rubato fails only on buffer-size misuse), so the test calls the reporting directly.
- `Pump::with_dither` removed; `Dither` and `convert_dithered` stay public.

checks: fmt pass, clippy pass, tests `cargo nextest run --workspace` × 3: 597 passed / 0 failed each (6 skipped); deny pass. Nothing left running.
files changed (this round): crates/tinwave-playback/src/decoder.rs, src/output/pump.rs, tests/common/mod.rs, tests/resampling.rs, tests/rt_alloc.rs, tests/failures.rs
signals added/changed: none
deviations: none beyond round 1.

proposals:
- P: Every test relying on a `replace_after` asserts its verdict via `Harness::expect_edit_applied` (pairing.rs:367 and the queue_edits tests still depend on timing) | size: S

questions for lead: none

---
Lead (2026-11-03): P-0232 merged → T-0056 (same test files; the late-append ticket will touch them anyway).
