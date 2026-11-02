HANDOFF T-0021 (round 1)
status: done
branch: t/T-0021-persist-signal-settings   commit: a15f379 (on main f28e113)
summary: Signal Center toggles survive a restart. signal_settings added to the database.md signals block and as migration 0002 (0001 untouched, pinned frozen). tinwave_db::PersistentSettings loads the row before the bus exists, applies each toggle at once then stores it, and reports an unusable row visibly. The stored mask layout is frozen in SignalSettings (explicit category numbers, mask() / from_mask()).

acceptance criteria:
- [x] Toggle off -> restart -> still off, zero events constructed: a_category_turned_off_stays_off_after_a_restart_and_nothing_is_constructed (each of 7 categories; counting constructor 0 constructed/queued; reopen => 0; other categories construct all kinds); tiers_and_several_toggles_are_stored_together; settings_can_be_saved_while_the_writer_is_running.
- [x] First run uses the documented defaults: the_first_run_uses_the_documented_defaults_and_stores_them (all categories on, T0/T1 on, T2/T3 off, mask 0x37F, updated_ts from the injected clock; reopen Restored).
- [x] fmt, clippy, tests, deny.

design: migration 0002_signal_settings.sql = lead's SQL; database.md block appended + append-only rule; the_migration_is_the_sql_block_of_database_md compares the doc block with all migrations in order; migration_0001_is_frozen pins 0001's checksum (-2299945054729597038 = main's); the_migrations_are_numbered_from_one. SignalCategory explicit discriminants 0..6 (stored bit numbers, never reused); mask()/from_mask(u32) refusing non-category/tier bits (SignalsError::InvalidValue); tests pin the layout (defaults 0x37F) and round trip. PersistentSettings::open(path, clock) migrates/loads/stores defaults on first run; settings() gives Arc<SignalSettings>; set_category/set_tier change the atomic mask first then upsert (mask read under the connection lock). a_toggle_writes_settings_and_never_a_signal. load() => Restored | FirstRun | Defaulted(problem); load_problem() => SettingsProblem (MissingRow, Corrupt{found}; Display text; logged at error); defaults in effect, row left as found. SQLite failure => DbError; failed save returns DbError while the toggle stays in effect. Upgrade (table created on a DB that already has signals) is a first run (relies on opening settings first).
checks: fmt pass, clippy pass, tests 378 passed / 0 failed / 6 ignored (none theirs), deny pass; release storage_growth 7 passed (extra table = one 4 KB page).
files changed: docs/rules/database.md; crates/tinwave-signals/src/settings.rs; crates/tinwave-db/{migrations/signals/0002_signal_settings.sql, src/{settings.rs (new), lib.rs, migrate.rs, store.rs}, tests/{signal_settings.rs (new), signal_store.rs}}
signals: none
proposals:
- P: Decide fail-closed vs defaults when the stored settings can't be read (defaults collect data a user may have switched off) | size: S
- P: Put the default row in migration 0002 itself (missing row always a fault; removes first-run/upgrade detection) | size: S
questions: none

---
Lead answers (2026-11-02): both proposals accepted before review — fail closed (all_off + visible error, "collection is paused until you review the settings"); default row (mask 895) inserted by migration 0002 and in database.md; table_is_new detection removed.
