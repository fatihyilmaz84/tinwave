-- signals.db, migration 0001. Source of truth: docs/rules/database.md ("Signal tables");
-- tests/signal_store.rs::the_migration_is_the_sql_block_of_database_md fails when they drift.
-- Pragmas (WAL, auto_vacuum, journal limit) are connection settings, not migrations: see src/connection.rs.

-- Dictionaries: each UUID stored once, referenced by small integers everywhere else.
CREATE TABLE dict_device    (id INTEGER PRIMARY KEY, uuid BLOB NOT NULL UNIQUE);
CREATE TABLE dict_recording (id INTEGER PRIMARY KEY, uuid BLOB NOT NULL UNIQUE);
CREATE TABLE dict_source    (id INTEGER PRIMARY KEY, uuid BLOB NOT NULL UNIQUE);
-- Albums, artists, playlists (UUIDv7 ids are unique across types; kind = EntityKind code, for readability/export).
CREATE TABLE dict_entity    (id INTEGER PRIMARY KEY, kind INTEGER NOT NULL, uuid BLOB NOT NULL UNIQUE);

CREATE TABLE session (
  id            INTEGER PRIMARY KEY,           -- assigned in start order, so ~time ordered
  uuid          BLOB NOT NULL UNIQUE,          -- SessionId (UUIDv7); needed for export and sync
  device        INTEGER NOT NULL REFERENCES dict_device(id),
  started_ts    INTEGER NOT NULL,           -- UTC ms
  tz_offset_min INTEGER NOT NULL
);

-- HOT: raw events, default 90 days. Target <= 60 bytes/row incl. index (realistic mix with ctx and a 50k-track
-- library: see T-0006 storage_budget test for the current measured figure).
-- WITHOUT ROWID keyed on the sync dedupe key: no rowid and no separate UNIQUE index (saves ~19 B/event).
CREATE TABLE signal_event (
  device    INTEGER NOT NULL,               -- with seq = global dedupe key for sync
  seq       INTEGER NOT NULL,               -- device-local sequence; has gaps, not a cursor
  session   INTEGER NOT NULL,
  ts_off    INTEGER NOT NULL,               -- ms since session.started_ts (absolute = started_ts + ts_off)
  kind      INTEGER NOT NULL,               -- SignalKind discriminant (stable, never reused); tier derived from kind
  recording INTEGER,
  source    INTEGER,
  pos_ds    INTEGER,                        -- playback position in deciseconds
  payload   BLOB,                           -- tiny packed struct, versioned per kind; NULL when empty
  ctx       INTEGER,                        -- context_snapshot id
  PRIMARY KEY (device, seq)
) WITHOUT ROWID;
-- No secondary index (lead decision, T-0006 review): nothing queries raw events by recording. Per-track heatmaps
-- and aggregates are built by the compaction pass, which scans in (device, seq) order; the taste engine and
-- Insights read `play` (indexed). Adding an index here needs a storage measurement against the 60 B budget.

CREATE TABLE context_snapshot (
  id     INTEGER PRIMARY KEY,
  hash   INTEGER NOT NULL UNIQUE,           -- 64-bit hash of quantized fields
  key    INTEGER NOT NULL,                  -- packed u32 taste-engine context key
  fields BLOB NOT NULL                      -- quantized ContextFields (temp as i8 °C, etc.)
);

-- WARM: one row per play, default 3 years.
CREATE TABLE play (
  id           INTEGER PRIMARY KEY,
  device       INTEGER NOT NULL,
  session      INTEGER NOT NULL,
  recording    INTEGER NOT NULL,
  source       INTEGER,
  started_ts   INTEGER NOT NULL,
  listened_ds  INTEGER NOT NULL,
  end_reason   INTEGER NOT NULL,            -- Completed | SkippedEarly | SkippedMid | SkippedLate | Stopped | Quit
  end_frac     INTEGER NOT NULL,            -- u8, per-255
  flags        INTEGER NOT NULL,            -- u16 bitset, see storage-budget.md
  peak_section INTEGER,
  context_key  INTEGER NOT NULL             -- packed u32
);
CREATE INDEX play_rec_ts ON play(recording, started_ts);

-- COLD: forever.
CREATE TABLE rollup_monthly (
  month       INTEGER NOT NULL,             -- yyyymm local
  recording   INTEGER NOT NULL,
  plays       INTEGER NOT NULL,
  completes   INTEGER NOT NULL,
  skips_early INTEGER NOT NULL,
  skips_late  INTEGER NOT NULL,
  fav_delta   INTEGER NOT NULL,
  listened_s  INTEGER NOT NULL,
  score_sum   REAL NOT NULL,
  PRIMARY KEY (month, recording)
) WITHOUT ROWID;

-- High-frequency UI signals are counters, never events.
CREATE TABLE ui_counter (
  day      INTEGER NOT NULL,                -- yyyymmdd local
  kind     INTEGER NOT NULL,                -- Impression | ArtworkHover | PageDwell
  entity_kind INTEGER NOT NULL,             -- EntityKind code: Track -> dict_recording, else dict_entity
  entity   INTEGER NOT NULL,                -- dictionary id in the table entity_kind selects
  count    INTEGER NOT NULL,
  total_ms INTEGER NOT NULL DEFAULT 0,
  max_scrolled INTEGER NOT NULL DEFAULT 0,  -- PageDwell only: running max scrolled fraction, quantized 0-255
  PRIMARY KEY (day, kind, entity_kind, entity)
) WITHOUT ROWID;
