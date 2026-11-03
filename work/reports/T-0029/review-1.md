REVIEW T-0029 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy pass (`-D warnings`, workspace, all targets); tests `cargo nextest run --workspace`: 644 passed, 0 failed, 6 skipped; deny pass (advisories, bans, licenses, sources ok). Ran with the Qt env in D:\dev\Tinwave-worktrees\T-0029-live-play-records @ 420dd6e.
Extra runs: `cargo test -p tinwave-db --test storage_growth -- --nocapture` passed 8/8. In debug the writer used 7.6 s of CPU against a 15 s budget (the machine was loaded by the other builders' cargo). In release (`--release`, ninety_days_of_a_heavy_listener...) it used 3.6 s against 8 s and passed.
Merge: `git merge-tree --write-tree main t/T-0029-live-play-records` is clean (tree eca337f). Since e46a240, main changed only docs/work files: ADR 0009, architecture.md, tinwave-specs.md, work/. No .rs, .toml or .sql file changed, and neither doc that tests `include_str!` (database.md, signal-catalog.md) changed. So the merged tree has the same code inputs as the branch, and the branch gate results apply to it.

acceptance criteria (judged per the lead rulings: Stopped/Quit are T-0059's; the all-unknown key is raw 0; context_key 0 in production until P-0236 lands):
- [x] Completed and Skipped* each write exactly one `play` row in the same transaction. Verified by:
  - `play_records::each_play_end_writes_exactly_one_row`: 4 outcomes give 4 rows; every column asserted.
  - `the_play_row_is_written_in_the_transaction_of_its_outcome`: a trigger makes the `play` insert fail; the batch's events roll back with it; the retry writes the row once; re-sending the batch writes nothing.
  - `the_writer_commits_the_play_row_with_the_play_end`: through the bus and the writer thread, with a 600 s flush interval.
  - Code: the outcome is collected only when its event is newly inserted (crates/tinwave-db/src/store.rs:342-357). `write_play` runs with `?` inside the same IMMEDIATE transaction before commit (store.rs:363-374). Dictionary caches are published only after commit (store.rs:411).
- [x] A failed track writes no play row. Verified by:
  - `a_failed_track_writes_no_row`.
  - The controller emits nothing on `TrackLeft{Failed}` (crates/tinwave-playback/src/controller/mod.rs:34), proven by the passing `controller_signals a_failed_track_is_neither_completed_nor_skipped`.
- [x] `ContextKey` has a documented, versioned layout; round-trip and unknown values are tested; a play with no context stores 0. Verified by:
  - The 8 tests in crates/tinwave-signals/src/context_key.rs: frozen `0x1015_9B76`, each field in its own bits, every value round-trips, unknown is 0, bad versions/reserved bits/codes are rejected.
  - `the_context_key_comes_from_the_snapshot_of_the_start_and_defaults_to_unknown`: no ctx, missing snapshot, unknown version and a negative key all store 0, and the outcome's snapshot is never used.
  - Layout matches the lead's decision: bits 0-2, 3-4, 5-7 + 8-10, 11-13, 14-17, 18-23, reserved 24-27, version 28-31. No enum overflows its field. OutputClass codes 0..6 +1 checked against enums.rs:316-325.
  - Documented in docs/guides/taste-engine.md "ContextKey layout (version 1)"; storage-budget.md links to it (anchor correct).
- [x] `peak_section` is NULL: the insert uses a literal NULL (crates/tinwave-db/src/play.rs:212), and it is asserted in `each_play_end_writes_exactly_one_row`.
- [x] Growth test accounts for play rows. Verified by:
  - `ninety_days_play_records_match_the_catalog_estimate` passes: 120.4 rows/day, 6963 B/day against 6000 (1.16x), rows equal simulated plays, no key-0 rows.
  - `the_catalog_still_says_what_the_test_expects` now also pins the catalog's "Play records" row.
  - 5-year extrapolation is 19.6 MB against 60 MB; 90-day hot window is 4.40 MB against 6 MB.
- [x] P-0111 (pairing per device by seq, never per session). Verified by:
  - `an_outcome_in_a_later_session_pairs_by_device_not_session` (separate batches, row keeps session 1, the pause is excluded).
  - `outcomes_pair_per_device` and `the_order_of_the_events_in_a_batch_does_not_matter`.
  - The lookup is `WHERE device=? AND seq<? ... ORDER BY seq DESC LIMIT 1` on the (device, seq) primary key (play.rs:195-199).
  - Across batches, order is guaranteed: PlayStarted, Skipped, Completed, Paused, Resumed, Favorited and SeekBack are all `Priority::Reserved` (crates/tinwave-signals/src/priority.rs:47-85), so they share one FIFO queue, and a failed batch is retried as is.
- [x] P-0127 tidy-ups.
  - settings.rs `ReleaseOnDrop` releases the parked thread on every exit path, so a failed assert no longer hangs the test.
  - The trigger literal in tests/signal_settings.rs:450 no longer has the run of spaces (see the non-blocking note).

blocking findings:
- none

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-db/src/play.rs:78-124 (`EndReason::of_skip`): the thresholds partly match the catalog's skip buckets.
  - The catalog (signal-catalog.md:53, taste-engine.md:26-31) has: < 5 s −1.0, 5–30 s −0.6, 30 s–50 % −0.2, > 80 % +0.2, and gives the 50–80 % band no weight.
  - The builder's two edges (30 s, > 80 %, i.e. raw > 204) are catalog edges, so nothing contradicts the catalog. But EndReason is coarser:
    - Early merges −1.0 and −0.6.
    - Mid merges 30 s–50 % (−0.2) with the unscored 50–80 % band.
    - For a short track that is both under 30 s and past 80 %, Late wins (the catalog doesn't say which should).
    - Time is measured as listened wall time minus pauses (consistent with how the controller computes `Completed.listened`, controller/mod.rs:108-110), not `pos_ds`. After a seek the two differ.
  - No information is lost: `listened_ds` and `end_frac` are stored, so every catalog bucket can be recomputed. The risk is a later consumer (taste engine, the monthly rollup's `skips_early/skips_late`) scoring by `end_reason` alone. P-0234 should write these rules into storage-budget.md/database.md, including that 50–80 % is Mid.
- crates/tinwave-db/tests/signal_settings.rs:450: the literal is now `"... signal_settings \n             BEGIN ..."` (an escaped newline followed by 13 spaces), not a `\` line continuation like the trigger literal a few lines above. The handoff calls it a "proper `\` continuation", which is inaccurate. It works; this is cosmetic only.
- crates/tinwave-db/src/play.rs:255-259: if the listener turns off the Queue & library category (which holds `PlayStarted`) while Seek/skip stays on, every outcome becomes unpaired and logs a warn. In the rare case where the last recorded start is an earlier, stopped play of the same recording, the new skip pairs with that old start and gets an inflated `listened_ds`. This is an edge case and toggles are respected at the source, so it is not blocking. T-0059's stop/quit signal will mostly close it.
- Measured warm tier: 2.5 MB/year (~58 B/row including the `play_rec_ts` index). The docs say ~2 MB/year (storage-budget.md:64, signal-catalog.md:182) and ~40–50 B/row (storage-budget.md:67). This is within the test's +50 % tolerance, but the prose is now optimistic.

proposals:
- P: Correct the warm-tier estimate in storage-budget.md/signal-catalog.md to the measured ~2.5 MB/year and ~58 B/row including the index | why: the growth test now measures it, and the docs say 2 MB and 40–50 B | size: S
- P: Wiping a T2/T3 category also masks the place/weather fields of `play.context_key` (the layout supports masking a field to 0) | why: privacy-and-signals.md:35 requires wipe per category/tier, and play rows now carry place slot and weather for 3 years | size: S
- P: When sync arrives, a start stored after its outcome must also trigger pairing (today only a newly stored outcome writes a row) | why: same-device order is guaranteed by the reserved FIFO, but synced or imported streams may not be | size: S

notes for lead:
- Safe to merge: merge-tree is clean, and the merged tree's code is identical to the branch's.
- Writer CPU is ~2x under budget in both profiles. My debug figure (7.6 s) is a bit above the builder's 7.0–7.4 s because the T-0027/T-0058 builders were compiling at the same time.
- The growth harness now inserts snapshot rows from the generator connection while the writer commits (busy timeout 10 s). The determinism tests (`the_same_seed_gives_the_same_sizes`, `the_rows_are_the_same_when_the_producer_is_preempted_after_a_pause`) passed.
- The replaced different-seed assertion compares stored event rows and play rows instead of file size. It is stronger: if the seed only changed UUIDs, the integer-id rows would still be equal and the assertion would fail.
- I made no scratch copies (none needed), the worktree is clean, and none of my processes are still running.

Relevant paths: D:\dev\Tinwave-worktrees\T-0029-live-play-records\crates\tinwave-db\src\play.rs, D:\dev\Tinwave-worktrees\T-0029-live-play-records\crates\tinwave-db\src\store.rs, D:\dev\Tinwave-worktrees\T-0029-live-play-records\crates\tinwave-signals\src\context_key.rs, D:\dev\Tinwave-worktrees\T-0029-live-play-records\crates\tinwave-db\tests\play_records.rs, D:\dev\Tinwave-worktrees\T-0029-live-play-records\crates\tinwave-db\tests\storage_growth.rs
