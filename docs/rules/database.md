# Database rules

## General

- SQLite in WAL mode, `foreign_keys=ON`, `synchronous=NORMAL`, `auto_vacuum=INCREMENTAL`,
  `journal_size_limit=16777216`. One writer connection per DB, pooled readers. Exception: signals.db's
  `signal_settings` row is upserted by `PersistentSettings` on its own short-lived transactions (single row, rare).
- Migrations are numbered SQL files in `crates/tinwave-db/migrations/`, applied in a transaction, never edited after release.
- Stable ids: UUIDv7 (time-ordered) for library entities created by Tinwave. Store external provider ids separately with provenance.
  **Exception:** inside `signals.db` use integer surrogate keys (see below) — size matters there.
- All queries paginated (keyset pagination on `(sort_key, id)`, not `OFFSET`).
- FTS5 for text search (artists, albums, titles, composers, playlist names).
- No binary blobs (audio, artwork) in ordinary rows. Artwork lives in a content-addressed cache dir.
- Every store has a size budget: see [storage-budget.md](storage-budget.md).

## Databases

| File | Contents |
|---|---|
| `library.db` | Library entities, playlists, sources, file ops journal |
| `signals.db` | Signal events (hot), play records (warm), monthly rollups (cold), UI counters, context snapshots |
| `taste.db` | Derived aggregates, track features, caches — rebuildable from `signals.db` at the granularity still retained |

Separating `signals.db` makes export/wipe simple and keeps the hot write path off the library DB.
`taste.db` can be deleted at any time and recomputed.

## Signal tables (signals.db)

```sql
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

-- HOT: raw events, default 90 days. Target <= 60 bytes/row incl. index. Measured (T-0006, realistic mix with ctx on
-- every event, 50k-track library): 39.4 B/event table, 48.2 B/event whole file; guarded by tinwave-db tests.
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

-- Migration 0002 (T-0021): Signal Center toggles, one row. Loaded before the bus accepts events.
CREATE TABLE signal_settings (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  mask       INTEGER NOT NULL,              -- SignalSettings category x tier bits (tinwave-signals settings.rs)
  updated_ts INTEGER NOT NULL               -- UTC ms
);
-- The documented defaults (every category on, T0 and T1 on, T2 and T3 off). A missing row is a fault, not a first run.
INSERT INTO signal_settings (id, mask, updated_ts) VALUES (1, 895, 0);
```

Migrations are append-only once merged to `main`: 0001 is frozen; new tables and changes go in the next number.

Play records are written **live** when a play ends (not only during compaction), so the warm tier is
always current and the taste engine can read plays without scanning raw events.

## Write path

- Signals go through `SignalWriter`: batch flush every **2 s or 256 events**, whichever first, plus on
  pause/stop/app quit. One transaction per batch. Dictionary lookups are cached in memory (bounded LRU).
- UI counters accumulate in memory (`HashMap<(kind, entity), (count, ms)>`) and are upserted with each batch.
- Crash safety: the in-memory batch is at most 2 s of data; acceptable loss. Play-end events and their
  play record are flushed immediately.
- Never `UPDATE` or `DELETE` a `signal_event` except for user wipe or compaction.
- Writer connection (T-0006, measured on Windows): `wal_autocheckpoint=0`; the writer runs
  `wal_checkpoint(TRUNCATE)` between batches — at 8 MiB when idle, at 64 MiB regardless, and every 60 s while the WAL
  is non-empty — never inside a commit and never waiting for a reader. SQLite's automatic checkpoint fsyncs for
  100–800 ms inside a commit on Windows. Writer page cache 256 KiB and a 2 048-entry dictionary LRU keep the writer
  under its 2 MB RSS budget (larger caches trade RSS for batch time).

## Retention & compaction

Hot raw 90 days → warm play records 3 years → cold monthly rollups forever (all configurable).
Compaction runs when idle, oldest first, in transactions of ≤ 5 000 rows, then `incremental_vacuum`.
Heatmaps and decayed aggregates in `taste.db` are updated before raw rows are deleted.
The budget enforcer may shorten windows; see [storage-budget.md](storage-budget.md).

## Reserved pragmas

- `signals.db` `PRAGMA user_version` is the **wipe epoch** (T-0065, `signal_wipe.rs` `wipe_epoch`): wipe everything
  bumps it inside its transaction and the writer drops its dictionary/session caches when it moves. Migrations use
  `schema_migration`, never `user_version`; do not reuse it.
