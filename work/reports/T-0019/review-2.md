REVIEW T-0019 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy -D warnings pass; tests pass (`cargo nextest run --workspace`, 597 passed, 6 skipped); deny pass (advisories, bans, licenses, sources ok). Worktree D:\dev\Tinwave-worktrees\T-0019-resampler-and-dither at d6d7e74.

Merge with main (54806d6): `git merge-tree --write-tree main t/T-0019-resampler-and-dither` gives c50c2f5 with no conflicts. I built the merged tree in a scratch copy made with `git archive`, using its own CARGO_TARGET_DIR and `--locked`. Results: `nextest --workspace` 622 passed, 6 skipped; clippy -D warnings pass; fmt pass. The merged root Cargo.toml has `cc = "1"` and `rubato = "5"`, and lib.rs has `mod resample;` and `pub mod stretch;`. I deleted the scratch copy and its target dir afterwards.

acceptance criteria:
- [x] 44.1 kHz pair on a 48 kHz endpoint stays gapless (<= 1 frame) — verified by: `resampling.rs::a_44k_pair_on_a_48k_stream_is_gapless_within_one_frame` and `the_boundary_between_same_rate_tracks_has_no_filter_restart` (round 1, unchanged). Both passed in all runs below.
- [x] Rate change between tracks plays without glitch and without callback allocation — verified by: `resampling.rs::rate_changes_between_tracks_play_without_a_glitch` (lag 0 at both joins; items 1 and 2 leave by rate-change flush with `LeaveCause::Finished` at exact frames, so a normal rate change is not marked failed). Also `rt_alloc.rs::render_never_allocates_across_rate_changes_and_resampled_seeks` (rt_alloc.rs:170-240). That test now:
  - starts on an 80,000-frame 44.1 kHz track (about 87,075 stream frames);
  - edits the tail after 8,820 played frames and asserts the edit applied (`expect_edit_applied`);
  - switches device and seeks inside the resampled track;
  - plays 44.1 -> 48 -> 44.1 -> 96 -> 48 kHz (long -> a48 -> b44 -> c96 -> a48) and past the end;
  - asserts 0 callback allocations, more than 190,000 real frames, and at least one starved callback.
  The test needs 470 waiting callbacks x 441 = 207,270 frames; 211,675 are available after the seek. In the first rt_alloc test both `replace_after` calls now run before anything plays and are asserted applied (rt_alloc.rs:84-101). The ring holds 48,000 of A's 48,777 frames, so the reader cannot have finished A.
- [x] Dither is applied only for integer outputs — verified by: convert.rs unit tests and `output_fake.rs::a_volume_below_one_is_dithered_only_on_the_integer_outputs_that_need_it` (round 1, unchanged; pass).
- [x] fmt, clippy `-D warnings`, tests, `cargo deny check` pass — verified by: the reviewer runs above, on the branch and on the merged tree.

Round-2 focus items:
1. Race fix (resampling.rs:263-295). p1 is 0..60,000 file frames (about 65,306 stream frames). At the edit the decoder can be at most ring (48,000) + played (4,800) + one pending chunk ahead, so it is still reading p1 and the edit can't be refused as stale. `Harness::expect_edit_applied` (tests/common/mod.rs:289-308) doesn't render, matches on the `EditId`, panics with the event list on `QueueEditRejected`, and is bounded (30,000 x 100 us).
   - Stress: 12 playback binaries in parallel (resampling, rt_alloc, failures, gapless, boundary, seek, queue_edits, pairing, engine, output_fake, append, position).
   - One run of 16 rounds with `--test-threads=4`, one run of 16 rounds with default threads: 384 binary runs, 384 `test result: ok`, 0 failures.
   - `a_queue_edit_keeps_the_resampler_running_across_the_new_tail` and the rt_alloc rate test showed `... ok` in every default-threads round (16/16 each). Round 1 had 7/60 failures.
2. rt_alloc: covered under the second criterion above; 0 allocations across all stress runs.
3. `failures.rs::seek_past_the_end_of_a_resampled_track_is_reported` (failures.rs:388-436). I re-ran the round-1 mutation in the scratch copy: decoder.rs:504 changed to `if file_frame > length =>`. The test now fails (panic at failures.rs:410); 176 passed, 1 failed. Source restored and verified identical.
4. `flush_failed` (decoder.rs:557-568) blames `st.last_opened` and sets `item_failed`. It is used at decoder.rs:530-533 (`use_rate`) and decoder.rs:593 (end of queue).
   - In `use_rate`, `last_opened` is still the previous track: it is only updated at decoder.rs:476, after `use_rate` returns. So the old track is blamed, and the new mark carries `prev_failed = true` into the old track's `TrackLeft`.
   - After `reset_stream`, both `resampler` and `last_opened` are None, so no flush happens.
   - A healthy track can't be marked failed: `flush_failed` runs only on `Err` from `StreamResampler::finish`, which errors only on rubato buffer-size misuse (resample.rs:200-210). The rate-change test asserts `Finished` for the flushed tracks.
   - Mutation: reverting the blame to `st.cur.checked_sub(1)` makes `decoder::tests::a_failed_flush_blames_the_last_track_that_was_read` fail. Restored.
5. `Pump::with_dither` is removed. `git grep -i with_dither` finds nothing in the branch; on main it appears only in work/reports. `Dither::new(Dither::DEFAULT_SEED)` is still used in `Pump::new` (pump.rs:103), and the convert.rs doc link points to `Pump::new`.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-playback/tests/rt_alloc.rs:102-104 — the comment still says "First fill: callbacks race the decoder, so some are starved". These 40 callbacks now come after two `expect_edit_applied` waits, during which the decoder has been filling the ring, so few or none are likely starved. The starved and underrun coverage now comes from the seek, device-switch and end-of-queue steps; the `starved_callbacks > 0` assert still holds. Only the comment is inaccurate.
- crates/tinwave-playback/src/decoder.rs:557-568 — on the bug-only flush-error path, if the last opened track had already ended with a decode error, a second `TrackFailed` is emitted for the same item. The `use_rate` wiring of `flush_failed` is tested only through the direct unit test, because the error can't be provoked. That is acceptable.
- crates/tinwave-playback/tests/rt_alloc.rs:221-223 — the 440 waiting callbacks leave only about 4,400 frames of margin before the queue ends. The test is deterministic, but it will panic in `wait_buffered` if fixture lengths in `rate_change_files` or the seek target change. A comment noting the budget would help.

proposals:
- none new (P-0232 → T-0056 already covers the remaining `replace_after` tests that depend on timing)

notes for lead: Ready to merge; the merge with main is clean and the merged tree passes fmt, clippy, workspace nextest and the `--locked` build. The worktree is clean at d6d7e74. My scratch folders under the scratchpad are deleted. Nothing I started is still running. A `python` process (PID 60496, started 2026-11-02 10:04) is still running; it is not the reviewer's and was already noted in round 1.
