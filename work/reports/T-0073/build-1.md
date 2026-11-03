HANDOFF T-0073 (round 1)
status: done
branch: t/T-0073-wipe-cost-and-lock-guards   commit: f4d1c00
summary: Both contention reference tests now send a second batch after the wipe returns. Two new tests count the SQL statements each wipe kind runs (rusqlite `trace_v2` on the wiper connection) and compare a small and a large database. The SECTIONED check is set-equal. The "idle interval" sentence no longer claims 60 s while a reader stays open. Three edits are outside the ticket scope (see deviations).

acceptance criteria:
- [x] Mutation: a per-row loop in a wipe fails a test. Proof: I added a loop in `signal_wipe::wipe_kinds` (an extra statement per play row, capped at 200). `a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` failed: "category Volume: the statements depend on the rows: 13 on the small database, 210 on the large one". I also added a per-distinct-key loop in `wipe.rs::rewrite_keys`. `a_context_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` failed for every category, e.g. "16 on the small database, 410 on the large one". Both mutations reverted, with `touch` so the mtime changed.
- [x] Mutation: a lock left open after a wipe fails a test. Proof: I added `conn.execute_batch("BEGIN IMMEDIATE")` after the checkpoint in all five wipe entry points. `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored` and `a_write_batch_during_a_wipe_of_the_reference_table_is_not_starved` both failed: "a batch sent after the wipe returned was not stored (5.4 s later): the wiper still holds a lock?". With the mutation only in `wipe_everything`, the Category and Tiers modes passed and the Everything mode failed. The old assertions (the first batch, `after_wipe`, `waited`) passed under the mutation, which confirms the T-0064 finding.
- [x] SECTIONED check set-equal. Proof: `the_source_signals_sit_in_the_categories_the_rule_documents` now checks both directions. With `SignalKind::Favorited` added to SECTIONED it failed with "Favorited is sectioned and belongs to QueueLibrary". The old "contains" check would have passed.
- [x] privacy-and-signals.md "idle interval" sentence corrected. The writer case keeps "at most its idle interval (60 s) later". The reader case now says the content stays in the main file and the WAL file while the reader stays open, and the first checkpoint after the reader ends its read transaction removes it.
- [x] fmt, clippy, nextest, deny pass — see checks.

Pinned statement counts, each read off the SQL and then confirmed by the test:
- Category: 9 for volume, speed, seek/skip/replay and browsing; 8 for queue/library, output/sound and session.
- Signal tier: T0 8, T1 8, T2 and T3 4.
- `wipe_tiers` (signals plus context): T0 8, T1 12, T2 and T3 8.
- Everything: 16.
- Context wipe: 8 per category, all categories at once, and tiers T1 to T3; 0 for tier T0 and for no category.

checks: fmt pass, clippy pass (`--workspace --all-targets -D warnings`, Qt env), tests 802 passed / 0 failed / 6 skipped (nextest --workspace, exit 0), deny pass (advisories, bans, licenses and sources ok)

files changed:
- crates/tinwave-db/tests/signal_wipe.rs
- crates/tinwave-db/tests/context_wipe.rs
- crates/tinwave-db/tests/common/mod.rs
- crates/tinwave-db/src/signal_wipe.rs (the SECTIONED test only; no production code changed)
- docs/rules/privacy-and-signals.md
- crates/tinwave-db/src/wipe.rs (outside the listed scope)
- crates/tinwave-db/Cargo.toml (outside the listed scope)

signals added/changed: none
ui: none
docs updated: docs/rules/privacy-and-signals.md (idle-interval sentence). I also fixed the matching doc comment on `checkpoint_best_effort` in wipe.rs.

deviations from ticket (three paths outside the ticket's `scope`):
1. crates/tinwave-db/Cargo.toml: dev-dependency `rusqlite = { workspace = true, features = ["trace"] }`. `trace_v2` is behind that feature and it was not enabled. The feature adds no dependencies and Cargo.lock is unchanged. It applies to test builds only.
2. crates/tinwave-db/src/wipe.rs: a `#[doc(hidden)] pub fn connection_for_tests(&mut self) -> &mut Connection` on `SignalWiper`. `#[cfg(test)]` cannot work here because the integration tests link the library without `cfg(test)`. Nothing in the product calls it.
3. The `checkpoint_best_effort` doc comment in wipe.rs said "Either way ... finishes on its next idle interval", the same overclaim as the privacy doc. I made it say what the privacy doc now says.

Other notes:
- Statement counts include BEGIN, COMMIT and the two `PRAGMA wal_checkpoint` calls, so a statement added to the checkpoint also trips the pin.
- `a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count` runs 16 wipes against a 3 000-row-per-table fixture. It takes a few seconds alone and about 69 s inside the full parallel nextest run, next to the storage_growth tests. It passed.
- The second batch uses a bare `write_batch`, not the retrying helper, and must be stored in under `AFTER_WIPE_SLACK` (5 s). With a leaked lock it fails after the 5 s busy timeout, so a failing run costs about 5 s.
- Build freshness: tinwave-db has no build.rs and nothing is written to OUT_DIR.

proposals:
- P: Remove the `connection_for_tests` accessor by moving the statement-count helper into a `#[cfg(test)]` module in wipe.rs | why: it keeps raw connection access out of the public (hidden) API | size: S | evidence: crates/tinwave-db/src/wipe.rs `connection_for_tests`; it needs a small fixture there, since the integration fixtures are not reachable from `src`
- P: Count statements for the writer's `write_batch` the same way, and pin it per batch size | why: the same per-row regression could happen in the hot write path, and a count test is cheaper than a timing test | size: S | evidence: `statements_of` in crates/tinwave-db/tests/common/mod.rs works for any connection once it exposes a `&mut Connection`

questions for lead:
- none. The three out-of-scope edits above are listed under deviations in case you want them handled differently. No processes of mine are left running; the cargo, clippy and python processes visible on the machine belong to the other builders.
