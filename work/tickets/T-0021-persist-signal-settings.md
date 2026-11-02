---
id: T-0021
title: Persist Signal Center toggles (SignalSettings) across restarts
type: feature
epic: E-01
status: in_review
priority: P1
size: S
depends_on: []          # T-0006, T-0007 done
scope:
  - docs/rules/database.md             # signals SQL block + migration rule (this ticket only)
  - crates/tinwave-signals/src/settings.rs
  - crates/tinwave-signals/src/lib.rs   # re-exports only
  - crates/tinwave-db/
owner: tw-builder (round 1, same builder as T-0006/T-0007)
branch: t/T-0021-persist-signal-settings
review_round: 1
created_by: from P-0031
---

## Context
T-0005's `SignalSettings` is an in-memory atomic mask (7 categories × T0–T3). docs/rules/privacy-and-signals.md:
toggles are checked at the source and must survive restarts; defaults T0, T1 on, T2, T3 off.

## Lead grooming (2026-11-02)
- Add this table to the signals SQL block of docs/rules/database.md **and** as migration **0002** in the same branch
  (0001 is frozen once on main; the guard `the_migration_is_the_sql_block_of_database_md` must then compare the doc block
  with all migrations in order). Also add after the block: "Migrations are append-only once merged to `main`: 0001 is
  frozen; new tables and changes go in the next number."

  ```sql
  -- Migration 0002 (T-0021): Signal Center toggles, one row. Loaded before the bus accepts events.
  CREATE TABLE signal_settings (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    mask       INTEGER NOT NULL,              -- SignalSettings category x tier bits (tinwave-signals settings.rs)
    updated_ts INTEGER NOT NULL               -- UTC ms
  );
  ```

## Scope
- A settings row (or small table) in signals.db; load at startup before the bus accepts events; save on change.
- Turning a category off is effective immediately and stays off after restart.

## Acceptance criteria
- [ ] Toggle off → restart → still off, and zero events of that category constructed — integration test.
- [ ] First run uses the documented defaults — test.
- [ ] fmt, clippy `-D warnings`, tests, `cargo deny check` pass.

## Signals & storage
None (toggle changes write settings, not signals — design-system.md `TwSignalRow`).
