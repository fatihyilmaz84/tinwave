-- signals.db, migration 0003 (T-0070). Source of truth: docs/rules/database.md ("Signal tables").
-- Migrations 0001 and 0002 are on main, so this column is a new number, not an edit.

-- Migration 0003 (T-0070): the time-weighted playback rate of the play (the derived RateSession), stored by the writer.
ALTER TABLE play ADD COLUMN rate INTEGER;   -- u8, hundredths minus 50 (tinwave-signals Rate); NULL = not measured
