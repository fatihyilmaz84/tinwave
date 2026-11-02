REVIEW T-0021 (round 1)
verdict: APPROVE
checks (run by reviewer at 81e0369): fmt pass, clippy pass, tests 379 passed / 0 failed / 6 ignored (none in this diff), deny pass. Release storage_growth: 7 passed in 12.9 s.

acceptance criteria:
- [x] Toggle off → restart → still off, zero events constructed — `a_category_turned_off_stays_off_after_a_restart_and_nothing_is_constructed` (each of 7 categories; counting constructor 0 constructed/queued now and after reopen; other categories construct all kinds); `tiers_and_several_toggles_are_stored_together`.
- [x] First run uses the documented defaults — `the_first_run_uses_the_documented_defaults_from_the_migrated_row` (all categories on, T0/T1 on, T2/T3 off, mask 895, updated_ts 0, untouched on reopen); `the_default_row_of_the_migration_is_the_default_mask` pins 895 to `SignalSettings::new().mask()`.
- [x] fmt, clippy, tests, deny.

focus items:
- Fail closed (settings.rs:139-158): missing row, text/blob/NULL, negative, > 32 bits, unknown bits ⇒ `all_off()` (mask 0, every category and tier off). Tests assert mask 0 and that a bus on those settings constructs and queues 0 in every category; UI counters gated at the source (tinwave-signals counters.rs:222); problem reported each start until a toggle is saved; logged at error with "collection is paused until you review the settings". Mutation "defaults instead of all_off on a fault": both fail-closed tests FAIL.
- No silent fallback: SQLite failures are DbError (read map_err, open/migrate propagate); only decodable-but-unusable values become SettingsProblem; failed save returns DbError, toggle stays in effect, old row untouched.
- Mask layout frozen: explicit discriminants 0..6; `from_mask` refuses bits outside categories/tiers (7, 12, 31, u32::MAX tested); `the_stored_mask_layout_is_frozen` pins every bit and 0x37F. Mutation "from_mask accepts unknown bits": corrupt-row test FAILS.
- Toggle concurrency correct by construction (settings.rs:189-205): atomic updated before taking the connection lock, upsert reads the mask inside the lock ⇒ the last save sees every earlier atomic update; row differs only if the last save fails (DbError). Stress (7 threads × 60 toggles + tier flips, 20 rounds): row == mask every time. The stress test did not catch a "read mask before the lock" mutation (window too small) — guarantee rests on reasoning (non-blocking).
- Migrations: 0001 untouched, `migration_0001_is_frozen` pins main's checksum (scratch edit FAILS); doc guard compares the block with all migrations in order (895→891 in database.md FAILS); upgrade path `a_database_from_before_the_settings_table_is_upgraded_with_the_defaults` (version-1 DB with an event gets 0002 + defaults row, no problem, no data lost); append-only rule sentence present.
- Toggle writes settings only (`a_toggle_writes_settings_and_never_a_signal`: signal_event, ui_counter, session, dict_recording empty after 15 toggles).
- Scope clean.

blocking findings:
- none

design check: n/a

non-blocking findings:
- settings.rs:166-172 — `load_problem()` never clears in-session; after a save the Signal Center would keep showing "collection is paused" until restart. Clear it on the first successful save, or document that the UI drops the banner.
- settings.rs:110-111 — PersistentSettings opens a second writing connection to signals.db (database.md: "one writer connection per DB"); a toggle can wait on the writer's short transactions (busy timeout up to 5 s), possibly on the UI thread later. Note the exception in database.md or route saves through the writer when the bridge lands.
- settings.rs:160-163 — `settings()` hands out the `Arc<SignalSettings>` whose own `set_category`/`set_tier` change the mask without persisting; document "toggle only through PersistentSettings" or hand producers a read-only view.
- No test pins the "mask read under the lock" invariant; optional deterministic test with a hook between the atomic update and the save.

proposals:
- P: Clear the "collection is paused" problem in-session after the first successful save | size: S

notes for lead: nothing blocks the merge; the one-writer-connection deviation is worth a line in database.md or a decision before the Signal Center bridge ticket. Scratch: rev-t0021-r1/, mut21.py.

---
Lead (2026-11-02): merged. Non-blocking 1, 3, 4 → T-0032; non-blocking 2 documented as an exception in database.md (P-0089).
