REVIEW T-0065 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy pass (-D warnings, workspace, all targets), tests pass on the merged tree (main f7d19fa + fcb3196, merge-tree f505145): first full nextest run 736/737 with 1 failure, the known `tinwave-desktop::image_provider cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` flake ("nothing was cancelled"; this branch has no desktop code); second full run 737 passed / 6 skipped, exit 0. deny pass (advisories, bans, licenses, sources ok).

**Merge and fresh builds:** the merge is clean, and the merged tree differs from fcb3196 only in work/. All builds ran in git-archive copies under the scratchpad, each with its own fresh CARGO_TARGET_DIR.
- One problem on my side, which affects how the builder's numbers can be read. My first CPU run shared one target dir between the pre and main copies. Cargo reused the pre binary for main, because path packages get the same hash in any checkout.
- I threw that run away and measured again with a separate target dir per copy. I also checked that each binary holds the right code: the main binary has "reading the data version", the branch binary has "reading the wipe epoch".

acceptance criteria:
- [x] Writer CPU back to the pre-T-0061 level (±10 %), and a wipe still invalidates the caches. Verified by my own 90-day storage_growth runs:
  - Debug writer CPU:
    - pre-T-0061 (959671b): 6.5, 6.2, 7.2, 6.4, 6.4 s (median 6.4)
    - main c5ebb15 (data_version): 9.5, 8.2, 8.6 s
    - branch: 5.9, 6.5, 6.7, 6.7, 6.6, 7.1 s (median 6.6, +3 %)
  - Release writer CPU:
    - pre-T-0061: 3.8, 3.8, 3.6 s
    - main: 4.5, 4.7, 4.9 s
    - branch: 3.7, 4.2, 4.0 s (+6 %)
    - The handoff says the old check cost nothing measurable in release. My runs show it cost about 25 % there too. Either way the fix removes it.
  - Mutations in scratch, each run on a fresh build:
    - Epoch check forced off: `a_store_with_warm_caches_writes_consistent_rows_after_a_wipe_of_everything`, `the_writer_thread_keeps_working_while_everything_is_wiped` and `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored` fail.
    - Always clear (`if true || …`): `a_commit_of_another_connection_that_is_not_a_wipe_keeps_the_writer_caches` fails.
  - Soundness of the epoch:
    - `PRAGMA user_version` is used only in signal_wipe.rs:273-290. Grep finds nothing in migrations/*.sql, migrate.rs, connection.rs or docs, so the migration runner does not use it.
    - It is bumped only by `wipe_everything`, inside its transaction. `a_failed_wipe_changes_nothing_and_works_once_the_fault_is_gone` asserts the epoch stays 0 after the failed wipe, so the bump rolls back with it.
    - The writer reads it inside the IMMEDIATE batch transaction, with `prepare_cached` (store.rs:306).
  - Category and context wipes need no invalidation. The caches hold only `dict_device/recording/source/entity` ids and `session` rows (dict.rs:23, 62). The only code that deletes those rows is the `EVERYTHING` list (signal_wipe.rs:231-235). Category wipes delete from signal_event, ui_counter, play and rollup_monthly; the context wipe rewrites keys. `only_a_wipe_of_everything_moves_the_wipe_epoch` covers this.
- [x] The seek wipe keeps `plays`, Queue & library clears it. Verified by:
  - `signal_wipe::wiping_a_category_deletes_exactly_its_events_and_the_data_derived_from_it` (plays 10/0/5/0, 3 rows cleared, 0 deleted) and `a_play_keeps_its_identity_unless_its_start_is_wiped` (0 rollup rows after the Queue & library wipe).
  - Mutation: putting `SEEK` back into the `plays` sources makes both tests fail.
  - The rule table (signal_wipe.rs:27) and privacy-and-signals.md:57 are updated.
- [x] The stale-start scenario writes no duplicate row. Verified by `play_records::an_outcome_cannot_pair_with_a_start_that_already_has_its_row_after_an_outcome_wipe`, which fails with the guard disabled.
  - The guard cannot drop a legitimate pairing under the T-0029 rules:
    - A start gets its row only in the batch that stores its first outcome. Any later outcome of that play has the first outcome as its nearest boundary, so it was already unpaired.
    - The key includes the device, so per-device pairing by seq is kept.
    - The session it compares is the start's own, so cross-session pairing is kept.
    - There is no time window, so paused plays still pair.
    - The T-0029 tests pass unchanged.
  - The only theoretical miss is two PlayStarted events of one recording in the same millisecond on one device and session, which cannot happen in practice.
  - The lookup uses the `play_rec_ts(recording, started_ts)` index (0001_init.sql:62), then filters on device and session.
- [x] The marker is absent from the main file right after the wipe returns. Verified by:
  - `wiped_content_is_not_left_in_the_database_file`: no manual checkpoint after the wipe; it expects (0, 0) for everything and (1, 0) for Volume. With the checkpoint made a no-op it fails.
  - `a_reader_that_blocks_the_checkpoint_does_not_fail_or_delay_the_wipe` is meaningful. The reader's snapshot is from before the wipe and the WAL was emptied first, so PASSIVE can copy nothing. With a 5 s busy timeout in the checkpoint it fails ("the wipe waited for the reader (6.08 s)").
  - The checkpoint runs PASSIVE then TRUNCATE with a zero busy timeout. Errors and busy results are only logged, the timeout is restored to `connection::BUSY_TIMEOUT`, and the wipe is already committed (wipe.rs:285-309).
- [x] fmt, clippy, nextest, deny pass: see checks above.

**Stability of the contention tests.** I added an eprintln of the batch wait to scratch copies of the branch and of main (c5ebb15).
- Normal load, `cargo nextest run -p tinwave-db` 5 times on the branch: 118/118 every time. Batch waits: signal category wipe 0.64-0.86 s, everything 0.64-0.76 s, context 0.25-0.32 s, against the 5 s busy timeout.
- 16 parallel copies of the signal test, 2 series of 4 rounds:
  - Branch: 128/128 pass. Category-wipe max 5.40 s in series 1 (median 2.4 s), 3.15 s in series 2 (median 1.45 s).
  - Main: 128/128 pass, max 3.21 s and 2.77 s (medians 1.6 s and 1.8 s).
  - So the series disagree on whether the branch is slower: noisy, no consistent regression.
- 16 parallel copies of the context test: branch max 0.87 s, main max 1.19 s, all pass.
- 16 signal + 16 context copies at once (32 heavy processes on 16 threads), 3 rounds:
  - Branch: 7/48 signal category-wipe runs failed with "database is locked" after about 5.5 s.
  - Main: 3/48 failed the same way under the same load.
  - Context runs never failed (max 3.25 s branch, 4.23 s main). The wipe-everything case never failed (max 1.5 s).
- My call: the failure under heavy oversubscription already exists on main and is not caused by the checkpoint. nextest runs one instance of each test, and there the headroom is about 6x. In the product a batch that waits out the timeout is retried, not lost. So I do not count it as blocking.

blocking findings:
- none

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-db/src/wipe.rs:301-303 and docs/rules/privacy-and-signals.md:69-70 — the busy result is logged as "a reader holds the WAL", and the doc names only that case. TRUNCATE also reports busy when the signal writer holds the write lock, which is likely right after a long wipe, because batches queue behind it. In that case the earlier PASSIVE has already put the zeroed pages in the main file. Only the WAL file keeps its pre-wipe frames until the writer's next checkpoint. The log and doc wording should say so.
- crates/tinwave-db/src/play.rs:284 — the guard covers a stale start that has a row. A stale start that never got a row (a stopped or failed play) can still pair with an orphan outcome of the same recording while Queue & library is off. This needs no wipe and is older than this ticket. The hot-window guard the ticket offered as an alternative would also cover it.
- crates/tinwave-db/tests/signal_wipe.rs:1027-1054 — the test does not bound or report `waited`, so a growing stall is invisible until it flakes (the builder's P already covers this).

proposals:
- P: Make the reference-size contention tests robust to machine load (assert the wait against the measured wipe time, or check that the writer's retry stores the batch) instead of relying on a fixed 5 s busy timeout | why: under 32 parallel heavy processes, 3/48 (main) and 7/48 (branch) category-wipe runs fail with "database is locked" | size: S
- P: Log the TRUNCATE busy cause correctly (writer vs reader) and say in the doc that only the WAL file keeps old frames when the writer holds the lock | why: the current message and doc misdescribe the common case right after a long wipe | size: S

notes for lead:
- The cancel_storm flake (T-0049/P-0169, considered fixed) came back once in a full-workspace run on this machine. It passed in the second run. It is unrelated to this branch.
- The handoff's "before" figure (8.3-8.6 s debug) matches my main measurement (8.2-9.5 s). Its release claim of no cost is off; I measured +25 % on main.
- Sharing a CARGO_TARGET_DIR between git-archive copies of the same workspace makes cargo silently reuse binaries across copies. Every copy needs its own target dir; the reviewer and builder rules should say so.
- Cleanup: I deleted everything I created (the r65 and r65-scripts folders in the session scratchpad, with all copies, target dirs and logs). Nothing of mine is running. I did not touch the T-0057 worktree.

