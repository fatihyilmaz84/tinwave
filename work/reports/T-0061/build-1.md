HANDOFF T-0061 (round 1)
status: done
branch: t/T-0061-signal-wipe-per-category-and-all   commit: be7311d
summary: `SignalWiper` gets `wipe_signal_categories(&[SignalCategory])` and `wipe_everything()`, each one IMMEDIATE transaction with a fixed number of single-scan statements, so it keeps the T-0060 cost discipline. The "derived from" rule (below) is a declarative table in `crates/tinwave-db/src/signal_wipe.rs` and is written into the privacy doc's wipe paragraph. The writer keeps working because `SignalStore` now drops its dictionary and session caches whenever another connection committed (`PRAGMA data_version`). Several rule choices need your decision before merge (Q1-Q4 below).

acceptance criteria:
- [x] Per-category wipe deletes exactly that category's events and derived data, tested per category. Proof: `signal_wipe::wiping_a_category_deletes_exactly_its_events_and_the_data_derived_from_it` loops over all 7 categories. It checks:
  - the surviving event kinds, including an unknown kind code, which only wipe-everything removes;
  - the counters;
  - expected play rows and rollup rows written out per category;
  - the exact `SignalWipeReport`;
  - that `context_snapshot`, `session`, the `dict_*` tables, `signal_settings` and `schema_migration` are unchanged.
  Also `several_categories_at_once_and_a_repeated_wipe` and `a_play_keeps_its_identity_unless_its_start_is_wiped`.
- [x] Wipe everything leaves empty signal tables and a working writer, including a batch in flight. Proof:
  - `wiping_everything_empties_the_signal_tables_and_keeps_the_settings` (10 tables empty, `signal_settings` and the schema kept, toggles survive a reopen).
  - `a_store_with_warm_caches_writes_consistent_rows_after_a_wipe_of_everything`.
  - `the_writer_thread_keeps_working_while_everything_is_wiped`: real bus plus `SignalWriter`, 600 events, wipe mid-stream. It asserts `failed_commits == 0`, `events_lost == 0`, wiped + stored == 600, and that every stored row resolves to its session, device and recording.
  - `a_write_batch_during_a_wipe_of_the_reference_tables_is_stored`: 131 400 plays, 40 000 events, 50 000 dict rows. It runs a category wipe and a wipe everything while the batch is issued, and the batch is stored.
- [x] Atomic under failure injection. Proof: `a_failed_wipe_changes_nothing_and_works_once_the_fault_is_gone`.
  - It injects `RAISE(ABORT)` triggers on the last table each wipe changes (rollups for category wipes, `dict_device` for wipe everything).
  - It compares a full dump of 12 tables before and after, then removes the trigger and checks the wipe works.
- [x] fmt, clippy, nextest, deny, storage growth. Proof: all pass (see checks).
- [x] Added from T-0060 review-2: the contention test asserts the probe saw the lock. `context_wipe::a_write_batch_during_a_wipe_of_the_reference_table_is_not_starved` now asserts `lock_seen`. It uses the new shared helpers `burn_in_trigger` and `during_a_wipe` in `tests/common/mod.rs`. A once-firing trigger burns about a 9M-row cross join inside the wipe, so the lock is held long enough on any machine. The same helpers are used by the new contention tests.

checks: fmt pass, clippy pass, tests 680 passed / 0 failed (6 skipped, as before), deny pass (advisories, bans, licenses, sources ok). The storage growth tests are in the 680 and pass.

Mutations tried (each undone):
- Disabling the `data_version` cache invalidation makes `a_store_with_warm_caches...` and `the_writer_thread_keeps_working...` fail with dangling session and recording ids.
- `secure_delete` set to false makes `wiped_content_is_not_left_in_the_database_file` fail (the marker stays in the file).
- The `tinwave-db` clippy run is clean.
- Not mutation-tested: the atomicity tests rely on a real injected abort, not on a code mutation, so that proof is by construction.

files changed (all under `crates/tinwave-db/` except the doc):
- `src/signal_wipe.rs` (new): the rule tables and SQL.
- `src/wipe.rs`: the two new methods, and `secure_delete=ON` on the wiper connection (this also applies to the T-0060 context wipes).
- `src/store.rs`: the `data_version` check per batch.
- `src/dict.rs`, `src/lru.rs`: `clear()`, plus an `Lru` unit test.
- `src/play.rs`: `EndReason::Unknown = 6`.
- `src/lib.rs`: exports `SignalWipeReport`.
- `tests/signal_wipe.rs` (new), `tests/common/mod.rs`, `tests/context_wipe.rs`.
- `docs/rules/privacy-and-signals.md` (wipe paragraph only).

signals added/changed: none (no new signal, no catalog rows, no storage change).
ui: none
docs updated: `docs/rules/privacy-and-signals.md` wipe paragraph.

The rule I implemented (my proposal; you decide). Principle: the T-0060 context rule, applied to signals. A derived column is cleared by the wipe of every category it is derived from, and nothing else changes. A derived row is deleted when what identifies it was wiped, or when its cleared columns were all it carried.
- A play row exists because a `PlayStarted` (catalog section 4, queue and library) names the recording and time.
  - Wiping queue and library deletes all `play` rows and all `rollup_monthly` rows.
  - Wiping seek/skip/replay keeps what was played and when. It sets `listened_ds=0`, `end_frac=0` and `end_reason=Unknown`, and clears the flag bits `replayed`, `seek_back` and `seek_fwd_intro`, plus `peak_section`.
  - Volume clears `vol_*` flags and `peak_section`. Speed clears `rate_*` flags and `peak_section`. Browsing clears the `lyrics` flag. Output/sound and session clear nothing in `play`.
- Rollups:
  - `completes`, `skips_*` and `listened_s` are cleared by seek/skip/replay.
  - `plays` is cleared by seek/skip/replay and by queue and library.
  - `fav_delta` is cleared by queue and library.
  - `score_sum` is reset to 0 by every category wipe.
  - A row left with only zero counters is deleted.
- A category wipe does not touch `context_snapshot`, `session`, the `dict_*` tables or `context_key`. Those are shared by all categories, or are context (handled by T-0060).
- Wipe everything deletes events, counters, play, rollup, snapshots, sessions and all four dictionaries. It keeps the schema and `signal_settings`. It covers `signals.db` only: `taste.db` does not exist, and the docs say so.

deviations from ticket:
- New public `EndReason::Unknown = 6`. The play table has NOT NULL outcome columns, and changing that would need a migration. Code 6 is additive and the existing codes are untouched. The play writer never writes it, and readers must treat it as "no information", not as a skip. `docs/rules/storage-budget.md` lists the end reasons and I could not edit it (out of scope), so see P1.
- I added `secure_delete=ON` to the wiper connection. The ticket did not ask for it, but a wipe that leaves content readable in the file's free pages is not a wipe. The cost is that deleted cells are zeroed inside the wipe's own transaction. A test greps the DB file for a marker.
- I added a `data_version` check to `SignalStore::write_batch`, which is outside the "wipe" code but inside `crates/tinwave-db/` scope. It is needed for "writer keeps working". A wipe of the dictionaries makes cached ids dangle, and SQLite hands out the integers of an emptied table again. That would silently attribute events to the wrong recording. The check is one pragma read per batch, under the write lock. A side effect: any other-connection commit (such as a settings toggle) also clears the caches, which only costs a few lookups.

proposals:
- P: Update `docs/rules/storage-budget.md` for `EndReason::Unknown` (code 6) and the wipe-cleared flag/column meaning | why: the doc enumerates end reasons and I was out of scope there | size: S | evidence: storage-budget.md "end_reason (T-0029 ...)" paragraph vs `crates/tinwave-db/src/play.rs`
- P: Persist a per-device high-water `seq` that survives wipe everything | why: after a wipe and restart `SignalStore::first_seq` returns 0, so a future sync peer holding `(device, 0..n)` would dedupe the new events away | size: M | evidence: `store.rs` `first_seq`; `wipe_everything` empties `signal_event`
- P: Wipe per tier for signal events (not only context) | why: the privacy doc promises "per category, per tier, everything"; only the context part of a tier exists, and the doc now says so | size: S | evidence: `kind.rs` `tier()` plus the `kind_codes` helper in `signal_wipe.rs`
- P: The compaction ticket (rollups) must register new `rollup_monthly` columns and `play.flags` sources in the `signal_wipe.rs` tables | why: the tables are the single place the rule lives; a unit test already requires an owner for each of flag bits 0-10 | size: S | evidence: `signal_wipe.rs` `FLAG_SOURCES`, `ROLLUP_COLUMNS`

questions for lead (need your decision before merge):
- Q1: Wiping queue and library deletes ALL play rows and rollups, because `PlayStarted` sits in that catalog section along with search, favorites and so on. The alternative is to keep rows, which needs a schema change to null out recording/start. I chose delete: an outcome with no recording or time carries no information. Confirm?
- Q2: Every category wipe resets `score_sum` in all monthly rollups to 0, following the doc's own precedent (derived data that blends categories is cleared, like `taste.db`). This loses cold-tier scores of other categories, for example a Browsing wipe zeroes the lifetime score. The alternative is to leave it and accept a residue. Nothing writes rollups yet, so it is one table entry in `signal_wipe.rs` if you want it changed.
- Q3: The seek/skip/replay wipe sets `end_reason` to the new `Unknown` code rather than deleting rows. That keeps what was played and when. OK?
- Q4: An unknown `kind` code (from a newer build) belongs to no category, so only wipe everything removes it. OK?
