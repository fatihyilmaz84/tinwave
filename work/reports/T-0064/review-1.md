REVIEW T-0064 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy pass (`--workspace --all-targets -D warnings`, Qt env); tests: nextest 797 passed / 0 failed / 6 skipped, exit 0; deny: advisories, bans, licenses and sources ok. Merged tree (main 4b54eb3 + branch, `git merge-tree` = 4cb0366, built in a scratch copy with its own target dir): no conflicts, fmt pass, clippy `-p tinwave-db -p tinwave-playback` pass, nextest for core/signals/context/db/playback 716 passed / 0 failed. Main's new code since the base is all in tinwave-playback (T-0067). It shares no files with this branch, and nothing outside tinwave-db uses `SignalWiper`.

acceptance criteria:
- [x] Per tier, exactly that tier's events and derived data go, with a test per tier — verified by: `wiping_a_tier_deletes_exactly_its_events_and_the_data_derived_from_it`. Each tier has literal expected reports and rows, and the test checks the context, session, dictionary, settings and migration tables are untouched. `several_tiers_at_once_and_a_repeated_wipe` checks that all tiers leave the same dump as all categories. kind.rs `tier()` gives T1 = WindowState(69), OutputDeviceChanged(81) and the derived PerceivedLevel; every other kind is T0, and catalog_sync `storage_class_and_tier_match_the_catalog` passes. The two lead decisions are in the code: T1 resets score_sum (`from: SignalKind::ALL`), and T0 deletes the play rows (ROW_IDENTITY = PlayStarted).
- [x] Category wipe gives the same result as before — verified by code and by tests. I mapped every derived column's source kinds back to categories (settings.rs `SignalCategory::of`):
  - OUTCOME is all in SeekSkipReplay.
  - SECTIONED covers exactly {Volume, Speed, SeekSkipReplay} (SystemVolumeChanged is Volume).
  - FLAG_SOURCES bits 0-10 land in the same categories as the old owners.
  - fav_delta is QueueLibrary, and score_sum covers all kinds, so every category.
  - `kinds.contains(PlayStarted)` is true exactly when QueueLibrary is wiped.
  - `kind_codes` keeps ALL order, so the SQL text is the same.
  - The pre-existing integration category tests are unchanged in the diff: the only changed hunks are the writer-thread and contention tests the ticket asked for, plus new tests. They pass.
- [x] Atomic, one call for signals plus context — verified by: `a_tier_wipe_is_one_call_for_the_signals_and_the_context_of_the_tier` (keys built with typed builders) and `a_failed_tier_wipe_changes_nothing_and_works_once_the_fault_is_gone`. The faults are injected at the real last statements:
  - combined call: the `UPDATE context_snapshot` of `rewrite_keys`, which runs after the signal part;
  - signal-only call: the final `UPDATE rollup_monthly` of `clear_rollups` for T1.
  - A full 12-table dump is identical after the failure.
- [x] Writer keeps working, using the contention helpers — verified by: `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored` (Category, Tiers T1+T2 over 131 400 plays, Everything), `the_writer_thread_keeps_working_while_everything_is_wiped`, and `a_batch_that_outwaits_the_busy_timeout_is_stored_by_the_retry`. The helper matches what the writer does (writer.rs:463-476: keep the stage, retry the same batch).
- [x] P-0272 / P-0280 / P-0277 (bounded waits, robust under load) — my 16+16 run, 3 rounds, branch binaries, with the T-0066/T-0067 reviewers also loading the machine:
  - signal reference test 48/48 pass, context test 48/48 pass.
  | case | wipe median / max | batch wait median / max | attempts max | after_wipe max |
  |---|---|---|---|---|
  | Category | 7.97 / 10.38 s | 2.41 / 7.51 s | 2 | 0 |
  | Tiers | 4.97 / 6.72 s | 1.33 / 4.66 s | 1 | 0 |
  | Everything | 5.37 / 10.93 s | 0.98 / 1.88 s | 1 | 0 |
  | Context | 6.52 / 10.03 s | 0.67 / 6.41 s | 2 | not parsed |
  - The waits above 5 s passed only because of the retry; the old bare `write_batch` would have failed them. The T-0065 review saw 7/48 failures (branch) and 3/48 (main) under the same load.
  - Writer-thread test: 16 copies plus 16 reference copies, 2 rounds: 32/32 pass, and every result line I printed showed failed_commits=0.
- [x] P-0281 (log and document both causes of a busy checkpoint) — verified by the two `wipe::tests` checkpoint tests. They are meaningful:
  - writer case: BEGIN IMMEDIATE, so PASSIVE copies everything and TRUNCATE is busy → `Busy{true}`;
  - reader case: an open read snapshot plus a later insert, so copied < log → `Busy{false}`;
  - both reach `Done` once released.
  - One edge case is wrong; see non-blocking finding 1.
- [x] fmt, clippy, nextest, deny pass — see checks.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-db/src/wipe.rs:378-385 — `main_file_current: copied >= log` ignores PASSIVE's own busy flag. If another connection holds the checkpoint lock (the writer's `checkpoint_if_due` TRUNCATE, which is likely right after a wipe has grown the WAL), SQLite returns busy=1 and log = copied = -1 without copying anything. -1 >= -1 then gives `Busy{main_file_current: true}`. So "true only when PASSIVE really copied every frame" does not hold in that race. Only the log line is affected. Fix: `let (passive_busy, log, copied) = …; main_file_current: passive_busy == 0 && copied >= log`.
- crates/tinwave-db/tests/common/mod.rs:441-447, tests/signal_wipe.rs:1592-1603 — the after-wipe bound can almost never trip:
  - In all 144 runs of mine, a waiting batch got the lock when the wipe committed, before the wipe even returned (after_wipe = 0).
  - A scratch mutation that leaves a `BEGIN IMMEDIATE` open on the wiper after the category wipe still passed the reference test, because the batch was already in. Only a later batch would be blocked.
  - `waited <= wipe_time + slack` holds by construction.
  - So the tests check liveness and that no batch is lost (meaningful, and they no longer depend on 5 s), but they do not bound the wipe's own cost. A per-row regression in the wipe SQL would not be caught. Cost discipline holds today by reading the SQL: wipe_kinds runs 2 deletes, then either 2 `DELETE FROM` or 1 play UPDATE plus a rollup DELETE and UPDATE. `wipe_fields_in` adds 2 × (1 scan + 1 UPDATE). All fixed, with no loops.
- crates/tinwave-db/src/signal_wipe.rs:36-39, docs/rules/privacy-and-signals.md:66 — a T0 wipe keeps the derived `PerceivedLevel` rows (T1), although they are computed from VolumeChanged and SystemVolumeChanged (T0). That does not match the "cleared by the wipe of any signal it is derived from" rule the module uses for score_sum. It is latent: nothing produces PerceivedLevel today.
- crates/tinwave-db/tests/signal_wipe.rs:133 — the fixture seeds only Event-class kinds, so the tier and category tests never contain a stored `RateSession` row. Those rows exist in production since T-0062. The code deletes them on T0 and Speed wipes, but no test proves it.
- crates/tinwave-db/src/signal_wipe.rs:540-547 — the SECTIONED check is "contains" rather than set-equal, so a future SECTIONED kind from another category would silently change that category's wipe.
- docs/rules/privacy-and-signals.md:80 — "at most its idle interval (60 s) later" does not hold in the reader case while the reader stays open. The wording predates this branch; the branch now also applies it to the reader case.

proposals:
- P: Pin the wipe's cost with a statement count (rusqlite `trace`/`profile` on the wiper connection), independent of row count | why: the contention tests no longer bound wipe time, so a per-row regression would go unnoticed | size: S
- P: Decide whether derived kinds go with their inputs' tier on a tier wipe (PerceivedLevel on T0), and seed a RateSession row in the wipe fixture | why: keeps the derived-data rule the same for score_sum and derived kinds before PerceivedLevel gets a producer | size: S
- P: Send a second batch after the wipe returns in the reference contention test | why: the only way the after-wipe bound can catch a lock left open by the wiper | size: S

notes for lead:
- Merge order: main gained T-0067 (playback) after this branch's base. The merge has no conflicts and the merged tree passes, so the branch can merge as is.
- Non-blocking finding 1 is a one-line fix that could be done before or after the merge.
- Scratch cleanup: I created only these files under C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad: sw_test.diff, nextest_branch.txt, deny.txt, load.ps1, load\, load2\, merged\, merged-target\, merged-clippy.txt, merged-nextest.txt, merged-mut.txt. All are deleted. I did not check for existing files before writing, so if load.ps1 or deny.txt already existed there, I overwrote and then deleted them.
- The worktree is unchanged at 104f688. None of my test processes or cargo runs are still going; the cargo and tinwave-desktop processes that are running belong to the other reviewers.
