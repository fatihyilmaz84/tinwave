REVIEW T-0061 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy pass (-D warnings, workspace, all targets), tests pass (worktree be7311d: nextest 680 passed / 0 failed / 6 skipped, exit 0; merged tree main+branch: 733 passed / 6 skipped, exit 0), deny pass (advisories, bans, licenses, sources ok, on both the worktree and the merged tree)

**Merge with main:** `git merge-tree --write-tree main be7311d` gives tree 90d7a6d with no conflicts. Main's T-0027 changes are in crates/tinwave-playback, work/, and docs (catalog §2, architecture.md, ADR 0006). None of them touch tinwave-db or the privacy doc. I archived the merged tree into a scratch copy with its own CARGO_TARGET_DIR and ran fmt, clippy, nextest and deny there: all green.

acceptance criteria:
- [x] Per-category wipe deletes exactly that category's events and derived data, tested per category.
  - Verified by `signal_wipe::wiping_a_category_deletes_exactly_its_events_and_the_data_derived_from_it`. It covers all 7 categories and checks exact play rows, exact rollup rows and the exact report. It also checks that snapshot, session, dictionary, settings and migration rows are unchanged, and that the play identity columns are unchanged.
  - Also `several_categories_at_once_and_a_repeated_wipe`, `a_play_keeps_its_identity_unless_its_start_is_wiped`, `an_unknown_kind_survives_every_category_wipe_and_not_wipe_everything`, and the unit tests in src/signal_wipe.rs (flag owners for bits 0–10, Unknown code pinned, outcome kinds are SeekSkipReplay).
  - I checked the tables against the catalog sections and `SignalCategory::of` (tinwave-signals/src/settings.rs:56). Flag bits: favorited, queued, removed_after → QueueLibrary; replayed, seek_back, seek_fwd_intro → SeekSkipReplay; vol_* → Volume; rate_* → Speed; lyrics → Browsing. Outcome columns → SeekSkipReplay. fav_delta → QueueLibrary. score_sum → all categories (Q2). Rows are deleted by QueueLibrary (Q1). Unknown kinds are removed only by wipe everything (Q4). I found no column that a category feeds and its wipe leaves behind.
- [x] Wipe everything leaves empty signal tables and a working writer, including a batch in flight.
  - `wiping_everything_empties_the_signal_tables_and_keeps_the_settings`: 10 tables empty; signal_settings, schema_migration and the toggle mask survive a reopen.
  - `a_store_with_warm_caches_writes_consistent_rows_after_a_wipe_of_everything`, `the_writer_thread_keeps_working_while_everything_is_wiped` (real bus and writer; wiped + stored == 600, 0 failed commits, 0 lost), and `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored`.
  - Mutation in scratch: forcing the data_version check off makes all three writer tests fail (dangling or wrong ids).
- [x] Atomic under failure injection. Verified by `a_failed_wipe_changes_nothing_and_works_once_the_fault_is_gone`: RAISE(ABORT) on the last table each wipe changes, a full 12-table dump is identical afterwards, and the wipe succeeds once the trigger is dropped.
- [x] fmt, clippy, nextest, deny; storage growth test passes. All run by me (above). The growth CPU check passes at 8.5–8.9 s against a 15 s debug budget.
- [x] Added from T-0060 review-2: the contention probe must have seen the lock. `during_a_wipe` returns `lock_seen` and both contention tests assert it (context_wipe.rs:766, signal_wipe.rs:929); the burn trigger keeps the lock held long enough.
  - Stability: I ran the two contention tests 6× in parallel together with the 90-day growth runs. The batch waited at most 0.99 s against the 5 s busy timeout (idle: 0.19–0.77 s), and the lock was seen in every run.

**Other points you asked about:**
- **Cost discipline:** each wipe is a fixed number of single-scan statements with no per-key or per-row loops. That is: `DELETE … WHERE kind IN (…)` on signal_event and on ui_counter, one `UPDATE play`, and `DELETE` + `UPDATE` on rollup_monthly. Wipe everything is 10 plain `DELETE FROM`.
- **data_version:**
  - It is read inside the IMMEDIATE transaction, so it is read under the write lock (store.rs:302).
  - No stale id can survive: the caches are cleared before the resolver is built, and they are only published after commit.
- **secure_delete:** set only on the wiper connection (wipe.rs:197); `connection::open` is unchanged. A scratch mutation setting it to false makes `wiped_content_is_not_left_in_the_database_file` fail.
- **EndReason::Unknown = 6:** additive, and codes 0–5 are unchanged. The frozen test is updated (`from_code(7)` is now None). The play writer never produces it (`write_play` uses only Completed or `of_skip`). Nothing outside tinwave-db reads end_reason.
- **Scope:** only crates/tinwave-db and the wipe bullet of privacy-and-signals.md are changed. No work/ edits.

blocking findings:
- none

design check: n/a (no UI)

non-blocking findings:
- crates/tinwave-db/tests/signal_wipe.rs:1 — the module doc comment is corrupted by a stray edit: `//! Wiping a signal catego.map(|r| [0, 1, 2, 3, 4, 5, 11].map(|i| r[i].clone()))y and wiping everything (T-0061).` It should read "Wiping a signal category and wiping everything (T-0061)." Please fix before or at merge.
- crates/tinwave-db/src/store.rs:1 and crates/tinwave-db/src/wipe.rs:1 — a UTF-8 BOM (EF BB BF) was added to both files; main has none. It shows up as a diff on line 1 and should be stripped.
- crates/tinwave-db/src/store.rs:302-306 — the data_version check costs much more than the handoff's "only costs a few lookups".
  - The growth harness writes context snapshots on its own connection (tests/growth/sim.rs:1129) about every 8 events, so the writer's caches are cleared before almost every batch: 4 347 clears in one 90-day run.
  - Writer CPU in the 90-day run (debug): main 6.2–6.9 s; branch 8.5–9.3 s; branch with the clear forced off 6.4–7.6 s. So the cache clears add about 1.7 s and the pragma read about 0.4 s, roughly +30% in total.
  - It is still well inside the 15 s budget. In production, database.md allows one writer connection, so clears would come only from wipes and settings toggles.
  - However, the growth test's speed guard now measures the uncached path. If the context hub ever writes snapshots on its own connection, as the harness models it, the dictionary cache is effectively off.
  - See proposal 1 for a cheap fix.
- crates/tinwave-db/src/signal_wipe.rs:171-175 (and privacy-and-signals.md:57) — the seek/skip/replay wipe zeroes `rollup_monthly.plays` but keeps `play` rows. Q3 says that wipe "keeps what was played and when", which holds for the 3-year tier but not the cold tier. The doc also says a rollup row left with zero counters is deleted, so in practice a seek wipe deletes almost every rollup row (only those with fav_delta survive). This is consistent with the code's own rule ("plays is derived from the outcome"), but it is a decision the lead has not explicitly made. Nothing writes rollups yet.
- docs/rules/privacy-and-signals.md:65 — "deleted content is zeroed in the file (`secure_delete`)" is only true after the next WAL checkpoint.
  - The wiper writes zeroed pages to the WAL. The main DB file keeps the plaintext until the writer's next TRUNCATE checkpoint (`wal_autocheckpoint=0`; idle interval 60 s, store.rs:76).
  - The marker test runs its own TRUNCATE checkpoint, so it proves the eventual state, not the moment the wipe returns.
  - Fix: either the wiper runs a best-effort `wal_checkpoint(TRUNCATE)` after commit, or the doc says "once the WAL is checkpointed".
- crates/tinwave-db/src/play.rs:203 (an interaction, not a changed line) — a seek/skip/replay wipe deletes every Completed and Skipped row, so every old PlayStarted becomes the "nearest boundary" again.
  - If an outcome's own PlayStarted is missing (for example the Queue & library category toggled off while Seek stays on), the first post-wipe outcome of the same recording pairs with a stale start.
  - That writes a second play row with an old started_ts and an inflated listened_ds.
  - It needs a specific sequence (seek wipe, then Queue & library toggled off, then that same recording's outcome first), so it is rare. It is worth a guard (for example, ignore a start older than the hot window, or skip a start that already has a play row) when the play writer is next touched.

proposals:
- P: Invalidate the writer caches only after a wipe, not after any other-connection commit | why: data_version fires on every foreign commit (4 347 clears and about +1.7 s writer CPU in the 90-day harness). Instead the wiper could bump a wipe epoch the writer reads in the same per-batch query, or on a version change the writer could probe that one cached `dict_device` id still maps to its UUID. Also use `prepare_cached` for the per-batch read | size: S
- P: Decide whether the seek/skip/replay wipe should keep `rollup_monthly.plays` | why: it would match Q3 ("keeps what was played") across the warm and cold tiers; today the cold tier loses play counts while the warm tier keeps them | size: S
- P: Guard play pairing against stale starts left by an outcome wipe | why: see the play.rs:203 finding; a stale start can produce a duplicate play row with a wrong listened_ds | size: S
- P: Wiper runs a TRUNCATE checkpoint after a wipe (best effort, zero busy timeout) | why: makes "zeroed in the file" true when the wipe returns, not up to about 60 s later | size: S

notes for lead:
- The change is about 580 production lines plus 1 100 test lines, against size M (≤ 400). Most of the production part is the rule tables and docs.
- Merge is clean. P-0255 (storage-budget.md for EndReason::Unknown) should land with or right after this merge, because the 0001_init.sql comment and storage-budget.md still list only codes 0–5.
- Events buffered in the bus at wipe time (up to one flush interval) are stored after the wipe, by design and tested. The Signal Center may want to flush the writer before a wipe so "wipe now" covers the last 2 s.
- Scratch cleanup: everything I created under D:\twrev61 is deleted (merged and main copies, their target dirs, logs). The empty folder D:\twrev61 itself is still there: the sandbox blocked removing a drive-root folder, so please remove it by hand.
- The cargo-nextest, tinwave_playback test binary and python processes still running belong to the T-0057 builder; I left them alone. Nothing of mine is running.

