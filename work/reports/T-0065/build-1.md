HANDOFF T-0065 (round 1)
status: done
branch: t/T-0065-wipe-follow-ups   commit: fcb3196
summary: The writer's caches are now cleared only by a wipe of everything, which bumps a wipe epoch (`PRAGMA user_version`, no new migration). The writer reads the epoch in its per-batch transaction with a cached statement. Debug writer CPU in the 90-day run is back from 8.3-8.6 s to 6.5-6.9 s. The other follow-ups are in too: the seek wipe keeps `rollup_monthly.plays`, a start that already has a play row is not paired again, and every wipe ends with a best-effort WAL checkpoint, so the marker is gone from the main file when the wipe returns.

acceptance criteria:
- [x] Writer CPU back to the pre-T-0061 level (+-10 %) and a wipe still invalidates the caches. Proof is the measurements below plus three mutations.
  - Debug writer CPU (storage_growth, 90 days):

| code | writer CPU (s) |
|---|---|
| before (main data_version) | 8.6, 8.4, 8.3 |
| pre-T-0061 equivalent (check removed) | 6.6, 6.7 |
| after (wipe epoch) | 6.5, 6.9, 6.5, 6.7 (one run at 7.5 right after a rebuild) |

  - Release writer CPU: before 4.3, 4.4; check removed 4.3; after 3.8, 3.5, 4.0. In release the old check cost nothing measurable, so it is unchanged within noise.
  - Mutation 1, epoch check forced off (`if false && ...`): `a_store_with_warm_caches_writes_consistent_rows_after_a_wipe_of_everything`, `the_writer_thread_keeps_working_while_everything_is_wiped` and `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored` all fail.
  - Mutation 2, always clear (`if true || ...`): new test `a_commit_of_another_connection_that_is_not_a_wipe_keeps_the_writer_caches` fails.
  - That new test does a settings toggle, a context snapshot insert and a category wipe on other connections, then changes a dictionary row under the cache. It proves the cached id is still used (no second `dict_recording` row).
  - New test `only_a_wipe_of_everything_moves_the_wipe_epoch`: category wipe, context wipe and settings toggle leave the epoch at 0; each wipe of everything adds 1.
  - `a_failed_wipe_changes_nothing_and_works_once_the_fault_is_gone` now also asserts the epoch is unchanged after the failed wipe and is 1 after the successful wipe of everything. So the bump rolls back with the wipe.
- [x] Seek wipe keeps `plays`, Queue & library clears it.
  - In `signal_wipe.rs`, `plays` is now derived from `[ROW_IDENTITY]` only.
  - `wiping_a_category_deletes_exactly_its_events_and_the_data_derived_from_it`: SeekSkipReplay now expects 4 rollup rows with plays [10, 0, 5, 0], 3 cleared, 0 deleted.
  - `a_play_keeps_its_identity_unless_its_start_is_wiped` checks plays [10,0,5,0] and no outcome counters after the seek wipe, then 0 rollup rows after the Queue & library wipe.
  - Mutation: putting `SEEK` back into the `plays` sources makes both tests fail.
- [x] Stale-start scenario writes no duplicate row.
  - Test `play_records::an_outcome_cannot_pair_with_a_start_that_already_has_its_row_after_an_outcome_wipe`: play, seek wipe, orphan outcome of the same recording, then a normal later play.
  - Orphan outcome: unpaired (plays_unpaired 1, plays_written 0), still 1 row with the old `started_ts`. The later play still pairs as the 2nd row.
  - Mutation: guard disabled makes the test fail.
  - Guard: `has_play_row` skips a start when a `play` row with the same recording, started_ts, device and session exists (indexed by `play_rec_ts`).
  - It cannot drop a legitimate pairing:
    - A row is written in the batch that stores the play's first outcome, so before that no row exists.
    - Any later outcome of the same play has the first outcome as its nearest boundary, so it was unpaired already.
    - There is no time window, so a play paused for hours still pairs.
    - The T-0029 tests are unchanged and pass: cross-session (`an_outcome_in_a_later_session_pairs_by_device_not_session`), `outcomes_pair_per_device`, and batch order.
- [x] The wiped marker is absent from the main DB file right after the wipe, with no manual checkpoint.
  - `wiped_content_is_not_left_in_the_database_file`: the manual `checkpoint()` after the wipe is gone. It asserts 0 copies of the marker in the main file and in the WAL for wipe everything. For the Volume wipe it asserts (1, 0): the snapshot blob's copy stays and the event payload copy is zeroed.
  - Mutation: with the checkpoint function a no-op, the test fails.
  - The wiper runs `PASSIVE` then `TRUNCATE` with a zero busy timeout. Errors and busy results are only logged, so the checkpoint never fails the wipe.
  - New test `a_reader_that_blocks_the_checkpoint_does_not_fail_or_delay_the_wipe`: an open pre-wipe read transaction means the wipe returns Ok in under 2 s and the marker is still in the file. After the reader commits, a checkpoint removes it. Mutation: a 5 s busy timeout in the checkpoint makes it fail (the wipe waited 5.5 s).
  - The doc caveat stays only for the blocked-reader case.
- [x] fmt, clippy, nextest, deny pass (below).

checks: fmt pass, clippy pass (-D warnings, workspace, all targets), tests 737 passed / 0 failed / 6 skipped (nextest --workspace, run twice, exit 0), deny pass (advisories, bans, licenses, sources ok)
files changed:
- /D:/dev/Tinwave-worktrees/T-0065-wipe-follow-ups/crates/tinwave-db/src/store.rs
- /D:/dev/Tinwave-worktrees/T-0065-wipe-follow-ups/crates/tinwave-db/src/signal_wipe.rs
- /D:/dev/Tinwave-worktrees/T-0065-wipe-follow-ups/crates/tinwave-db/src/wipe.rs
- /D:/dev/Tinwave-worktrees/T-0065-wipe-follow-ups/crates/tinwave-db/src/play.rs
- /D:/dev/Tinwave-worktrees/T-0065-wipe-follow-ups/crates/tinwave-db/tests/signal_wipe.rs
- /D:/dev/Tinwave-worktrees/T-0065-wipe-follow-ups/crates/tinwave-db/tests/play_records.rs
- /D:/dev/Tinwave-worktrees/T-0065-wipe-follow-ups/docs/rules/privacy-and-signals.md (wipe paragraph only)
- No BOMs in any changed file.
signals added/changed: none
ui: none
docs updated:
- Wipe paragraph of privacy-and-signals.md: `plays` rule, the checkpoint wording with the blocked-reader caveat, "wipe everything makes the writer drop its caches", and the unpaired-outcome case.
- Module docs of signal_wipe.rs (rule table row for `plays`, the wipe epoch, the checkpoint) and play.rs ("A start pairs once").
deviations from ticket:
- The wipe epoch lives in `PRAGMA user_version` rather than a table. A table would need migration 0003, which changes the table-list/migration-count tests and database.md, which is out of scope. `user_version` was unused (grep: no other use). Only wipe everything bumps it, because the category and context wipes never delete the dictionaries or sessions the caches point at. The bump is inside the wipe transaction and rolls back with it.
- The checkpoint runs `PASSIVE` before `TRUNCATE`; the ticket named only `TRUNCATE`.
  - A `TRUNCATE`-only checkpoint holds the write lock for the whole copy. After the 131 400-row reference wipe it made the writer wait about 1.0-1.7 s instead of 0.65-0.77 s, and it made `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored` flake under a loaded full-workspace run, probably because the writer's 5 s busy timeout ran out. I saw that failure once and did not capture the message.
  - With `PASSIVE` first, the writer waited 0.65-0.77 s in the quiet runs and the full workspace run passed twice.
- The checkpoint runs after all three wipe kinds (context wipe too), not only the signal wipes. Same `secure_delete` reason.

proposals:
- P: Note `PRAGMA user_version` as the signals.db wipe epoch in docs/rules/database.md | why: a future migration author must not reuse it; this ticket could not edit that doc (scope) | size: S | evidence: crates/tinwave-db/src/signal_wipe.rs `wipe_epoch`
- P: Have the wiper nudge the writer's `min_gap` or checkpoint clock after a wipe | why: after the wiper's checkpoint the writer's next idle checkpoint of an empty WAL is a no-op, but its 60 s timer is still running | size: S | evidence: store.rs `checkpoint_if_due`
- P: Make the checkpoint-stall contention test assert a bound on `waited` | why: the test only proves the batch succeeded within 5 s, so a growing stall is invisible until it flakes | size: S | evidence: tests/signal_wipe.rs `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored` (waited 0.65-0.77 s quiet, up to 3.3 s with 4 parallel copies)

questions for lead:
- none

Process note: for a while I measured and tested a stale build. I swapped store.rs back and forth with `Copy-Item`, which keeps the old mtime, so cargo did not rebuild. I found it when 3 runs failed and then passed, redid every measurement and gate after touching the files, and the numbers above are from fresh builds. Nothing of mine is still running. The cargo/nextest/python/tinwave_playback processes in the process list belong to the T-0057 builder.
