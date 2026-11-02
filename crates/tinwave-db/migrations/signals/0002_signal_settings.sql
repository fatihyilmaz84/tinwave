-- signals.db, migration 0002 (T-0021). Source of truth: docs/rules/database.md ("Signal tables").
-- Migration 0001 is frozen: it is on main, so this table is a new number, not an edit.

-- Migration 0002 (T-0021): Signal Center toggles, one row. Loaded before the bus accepts events.
CREATE TABLE signal_settings (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  mask       INTEGER NOT NULL,              -- SignalSettings category x tier bits (tinwave-signals settings.rs)
  updated_ts INTEGER NOT NULL               -- UTC ms
);
-- The documented defaults (every category on, T0 and T1 on, T2 and T3 off). A missing row is a fault, not a first run.
INSERT INTO signal_settings (id, mask, updated_ts) VALUES (1, 895, 0);
