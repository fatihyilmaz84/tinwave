HANDOFF T-0060 (round 2)
status: done (the context_snapshot.fields blob wipe stays open and out of scope, per the lead)
branch: t/T-0060-wipe-masks-context-key   commit: 6264ede (on top of 209413c, 7d581e6)
summary: Round 2 fixes the blocking finding. `rewrite_keys` no longer runs one unindexed full-table `UPDATE` per distinct key. Each table now gets one scan that checks for stored keys that are not a `u32`, then one `UPDATE` that does `ContextKey::clear_raw` in SQL, so the write lock is held for about a scan whatever the number of distinct keys. I added a randomized semantic test and a lock-contention test at the reference table size. I updated the module docs as the reviewer asked.

The fix, in crates/tinwave-db/src/wipe.rs:
- Out-of-range check: first `SELECT {col} FROM {table} WHERE typeof({col}) != 'integer' OR {col} < 0 OR {col} > 4294967295 LIMIT 1`. A hit returns `DbError::OutOfRange` and rolls back, as before. Text or real values count as out of range too.
- Rewrite: `UPDATE {t} SET {c} = CASE WHEN ({c} & keep) & region = 0 THEN 0 ELSE {c} & keep END WHERE {c} != 0 AND (({c} & clear) != 0 OR ({c} & region) = 0)`.
- Bit arithmetic: `clear` is `fields.bits()`, `keep` is `0xFFFF_FFFF ^ clear`, and `region` is the field bits 0-23.
- Match with `clear_raw`: the result is `raw & keep`, or 0 when no field bit is left. A row changes when it has a bit to clear, or when it is a non-zero key with no field bit at all, such as a version nibble alone, which `clear_raw` turns into 0. A reviewer-suggested WHERE with only `(c & clear) != 0` would have left version-only keys unchanged, which differs from `clear_raw`, so I did not use it.
- No index was added on `context_key`.
- Signals: `ContextFieldSet::bits()` is now `pub`, and I added `ContextFieldSet::ALL`; its `bits()` is the field region. Both are in context_key.rs, and a test pins `ALL.bits()` to the 24 field bits.

acceptance criteria:
- [x] Wiping each T2/T3 context category zeroes exactly its ContextKey fields (owned plus derived) in every play row; other fields untouched. Proof: the round 1 tests still pass: `wiping_a_category_clears_exactly_its_fields_in_every_play_row`, `wiping_a_tier_clears_the_fields_of_its_categories`, `wiping_the_place_or_the_clock_also_clears_the_derived_activity`, `several_categories_at_once_and_a_second_wipe_changes_nothing`, `many_rows_with_the_same_key_are_all_rewritten` and the end-to-end test.
- [x] A key whose last known field is wiped becomes raw 0. Proof: `a_key_whose_last_field_is_wiped_becomes_raw_zero` and `clearing_the_last_known_field_gives_raw_zero`.
- [x] The wipe is atomic. Proof: `a_failure_after_the_play_rows_were_rewritten_changes_nothing`, `a_failure_in_the_middle_of_the_play_rewrite_changes_nothing` and `a_stored_key_that_is_not_a_u32_fails_the_wipe_and_changes_nothing` pass with the new SQL.
- [x] Turning a category off stops new values only. Proof: `turning_a_category_off_leaves_existing_rows_alone`.
- [x] Review test 1, semantics pinned against `clear_raw`: `many_distinct_keys_are_rewritten_exactly_as_clear_raw_does`. It uses about 600 random valid keys plus 7 odd keys (a version nibble alone, reserved bits, version 2, 0xFFFFFFFF, 0xDEADBEEF and others), 3,000 rows in each table, and every category and tier T1 to T3. Both tables are compared row by row with `ContextKey::clear_raw`, and the report counts must match.
- [x] Review test 2, timing: `a_write_batch_during_a_wipe_of_the_reference_table_is_not_starved` builds 131,400 plays and about 4,000 distinct keys, which is at least 1,500 and normally close to 4,000. A thread runs the NetworkPlace wipe. A probe connection waits until it sees the wipe's write lock, then `SignalStore::write_batch` (5 s busy timeout) must succeed and write the play. The whole table is then compared with `clear_raw`. It takes about 1.5 s in the default debug run, so no size gating was needed.
- [x] Review item 3, module doc claim: the false "handful of statements" text is replaced in the `rewrite_keys` doc and in a new "Cost and locking" section of the module docs. They state one check scan plus one update scan per table, no index, and why.
- [x] fmt, clippy, nextest, deny pass; storage growth tests still pass. Proof: fmt exit 0, clippy `-D warnings` exit 0, nextest "665 tests run: 665 passed, 6 skipped", `cargo deny check` reports "advisories ok, bans ok, licenses ok, sources ok".
- [ ] `context_snapshot.fields` blob wipe — not done; accepted by the lead as out of scope for the encoding ticket.

Revert tests:
- I temporarily put the per-key loop back in `rewrite_keys`, and the timing test fails. The batch failed after 5.57 s with "sqlite error while starting the batch transaction: database is locked", while the wipe took 56.6 s in debug.
- The semantic test still passed with the old loop, which is correct because the old loop was also correct.
- I dropped the `OR ({c} & region) = 0` clause from the SQL, and the semantic test failed.
- All files are restored, and the commit contains the final SQL.

checks: fmt pass, clippy pass, tests 665 passed / 0 failed, deny pass
files changed:
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\src\wipe.rs (`rewrite_keys`, module docs)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\tests\context_wipe.rs (bulk fixture in one transaction, 2 new tests, 13 tests in total)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-signals\src\context_key.rs (`ContextFieldSet::bits` made pub, `ContextFieldSet::ALL`, test)
signals added/changed: none
ui: none
docs updated: wipe.rs module docs only this round. They now carry the cost section, a note that idle, fullscreen and power must join `derived_fields` if they become wipeable categories, and a note that `context_snapshot.hash` diverges from a wiped key, parked with the blob work.
deviations from ticket: unchanged from round 1 (built `SignalWiper`; re-export edit in tinwave-signals lib.rs). This round also makes `ContextFieldSet::bits` public and adds `ContextFieldSet::ALL` in context_key.rs, which is inside the scope of "a field-mask helper".

proposals:
- P: Wire the Signal Center wipe action to `SignalWiper` | why: nothing calls it yet | size: S | evidence: crates/tinwave-db/src/wipe.rs
- P: The context hub ticket writes snapshot rows only through the writer transaction, and handles hash/key divergence after a wipe (reviewer's P) | why: a post-wipe snapshot insert built from pre-wipe values would reintroduce wiped fields | size: S | evidence: wipe.rs module docs, "Not yet covered"
- P: Wipe-everything and per-signal-category wipe (`signal_event` rows, `taste.db`) | why: privacy-and-signals.md promises "per category, per tier, everything" | size: M | evidence: docs/rules/privacy-and-signals.md

questions for lead: none

Process note: nothing I started is still running. A final check found no tinwave-desktop or image-stress.
