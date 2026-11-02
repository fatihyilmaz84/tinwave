//! `SignalStore`: the signals.db write path, one transaction per batch.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tinwave_core::{DeviceId, SessionId, TzOffsetMin};
use tinwave_signals::{CounterRow, EntityKind, SignalEvent, SignalKind, SignalSink, SignalsError};

use crate::connection;
use crate::dict::{DictCaches, Resolver, Table};
use crate::migrate::{migrate, Migration, MigrationReport};
use crate::DbError;

/// The migrations of `signals.db`, oldest first.
pub const SIGNALS_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "init",
    sql: include_str!("../migrations/signals/0001_init.sql"),
}];

const INSERT_EVENT: &str = "INSERT OR IGNORE INTO signal_event \
    (device, seq, ts, session, kind, recording, source, pos_ds, payload, ctx) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)";

const INSERT_SESSION: &str =
    "INSERT INTO session (device, started_ts, tz_offset_min) VALUES (?1, ?2, ?3)";

/// Counter rows are deltas: add the count and the milliseconds, keep the larger scroll depth.
const UPSERT_COUNTER: &str = "INSERT INTO ui_counter \
    (day, kind, entity_kind, entity, count, total_ms, max_scrolled) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
    ON CONFLICT (day, kind, entity_kind, entity) DO UPDATE SET \
    count = count + excluded.count, \
    total_ms = total_ms + excluded.total_ms, \
    max_scrolled = max(max_scrolled, excluded.max_scrolled)";

/// The WAL size at which an idle writer checkpoints: 8 MiB.
const WAL_SOFT_LIMIT_BYTES: u64 = 8 * 1024 * 1024;
/// The WAL size at which a checkpoint happens even if events are waiting: 64 MiB. Bounds the
/// disk the WAL can take (and the time a crash recovery replays) under an unbroken flood.
const WAL_HARD_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
/// An idle writer checkpoints a non-empty WAL at least this often. With `synchronous=NORMAL`
/// a commit is not fsynced, so this bounds what a power loss (not an application crash) can
/// take: a typical listener commits a few pages every few minutes and would otherwise sit
/// below the size limit for hours.
const WAL_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(60);
/// Shortest time between two checkpoint attempts that are triggered by size or age, so a reader
/// that keeps the WAL busy does not make every commit retry (and fsync).
const WAL_CHECKPOINT_MIN_GAP: Duration = Duration::from_secs(2);

/// Sizes of the in-memory caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreConfig {
    /// Dictionary entries (UUID to integer) kept in the LRU, across all dictionaries. Each
    /// entry is about 70 bytes, so the default of 2 048 is about 150 KB. A miss costs one
    /// indexed `SELECT`, so the size trades memory for a little read work.
    pub dict_cache_capacity: usize,
    /// Sessions kept in the LRU. A session is rare (one per 30 minutes of silence).
    pub session_cache_capacity: usize,
    /// SQLite page cache of the writer connection, in KiB. SQLite's own default is 2 MiB,
    /// which alone would use the whole "writer adds < 2 MB RSS" budget (`testing.md`).
    /// Measured (100k events, Windows): 256 KiB gives a 256-event batch in about 3 ms and
    /// +1.5 MB RSS; 1 MiB gives 2 ms and +2.9 MB; 128 KiB gives 3.5 ms.
    pub page_cache_kib: u32,
    /// Timezone offset recorded on session rows until [`SessionTz::set`] changes it.
    pub tz_offset: TzOffsetMin,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            dict_cache_capacity: 2_048,
            session_cache_capacity: 256,
            page_cache_kib: 256,
            tz_offset: TzOffsetMin::UTC,
        }
    }
}

/// The timezone offset the store writes on new `session` rows. Shared and cloneable so the
/// session starter can set it from any thread while the writer owns the store; the same moment
/// it calls `SignalBus::set_tz_offset`.
#[derive(Debug, Clone)]
pub struct SessionTz(Arc<AtomicI16>);

impl SessionTz {
    fn new(tz: TzOffsetMin) -> Self {
        Self(Arc::new(AtomicI16::new(tz.minutes())))
    }

    /// Sets the offset for sessions first seen from now on.
    pub fn set(&self, tz: TzOffsetMin) {
        self.0.store(tz.minutes(), Ordering::Relaxed);
    }

    /// The current offset.
    #[must_use]
    pub fn get(&self) -> TzOffsetMin {
        // Only ever stored from a validated `TzOffsetMin`.
        TzOffsetMin::new(self.0.load(Ordering::Relaxed)).unwrap_or(TzOffsetMin::UTC)
    }
}

/// What one committed batch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchReport {
    /// Events inserted.
    pub events_inserted: usize,
    /// Events skipped because `(device, seq)` was already stored (sync dedupe, or a retry).
    pub events_duplicate: usize,
    /// Events dropped because their payload could not be encoded (a producer bug; logged).
    pub events_rejected: usize,
    /// Counter rows added into `ui_counter`.
    pub counter_rows: usize,
}

/// The signals.db writer connection: schema, dictionaries and the batch write.
///
/// One `SignalStore` owns the single writer connection of the file. It implements
/// [`SignalSink`] for the bus and keeps the typed [`DbError`] of the last failure
/// ([`last_error`](Self::last_error)); [`write_batch`](Self::write_batch) returns it directly.
pub struct SignalStore {
    conn: Connection,
    wal_path: PathBuf,
    last_checkpoint: Instant,
    caches: DictCaches,
    tz: SessionTz,
    last_error: Option<DbError>,
    last_report: BatchReport,
}

impl std::fmt::Debug for SignalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalStore")
            .field("last_report", &self.last_report)
            .finish_non_exhaustive()
    }
}

impl SignalStore {
    /// Opens `signals.db` at `path` (creating it), applies the pragmas of
    /// `docs/rules/database.md` and the migrations. Idempotent on an existing file.
    ///
    /// # Errors
    /// See [`DbError`].
    pub fn open(path: &Path) -> Result<Self, DbError> {
        Self::open_with(path, StoreConfig::default())
    }

    /// [`open`](Self::open) with explicit cache sizes and timezone.
    ///
    /// # Errors
    /// See [`DbError`].
    pub fn open_with(path: &Path, config: StoreConfig) -> Result<Self, DbError> {
        let mut conn = connection::open(path, config.page_cache_kib)?;
        migrate(&mut conn, SIGNALS_MIGRATIONS)?;
        let mut wal_path = path.as_os_str().to_owned();
        wal_path.push("-wal");
        Ok(Self {
            conn,
            wal_path: PathBuf::from(wal_path),
            last_checkpoint: Instant::now(),
            caches: DictCaches::new(config.dict_cache_capacity, config.session_cache_capacity),
            tz: SessionTz::new(config.tz_offset),
            last_error: None,
            last_report: BatchReport::default(),
        })
    }

    /// What the migration runner finds on this database (always up to date after `open`).
    ///
    /// # Errors
    /// See [`migrate`].
    pub fn migrate(&mut self) -> Result<MigrationReport, DbError> {
        migrate(&mut self.conn, SIGNALS_MIGRATIONS)
    }

    /// The handle that sets the timezone offset for new sessions.
    #[must_use]
    pub fn session_tz(&self) -> SessionTz {
        self.tz.clone()
    }

    /// The `seq` the bus must start at: one more than the highest stored `seq` of `device`, or
    /// 0 when it has none. Pass it as `BusConfig::first_seq` so a new run cannot reuse a `seq`.
    ///
    /// This is the one legitimate use of the highest stored `seq`; it is not a sync cursor
    /// (`seq`s have gaps and commit slightly out of order, see the bus docs).
    ///
    /// # Errors
    /// [`DbError::Sqlite`] on a database failure; [`DbError::OutOfRange`] if the stored maximum
    /// is not a valid `seq`.
    pub fn first_seq(&self, device: DeviceId) -> Result<u64, DbError> {
        let max: Option<i64> = self
            .conn
            .query_row(
                "SELECT max(seq) FROM signal_event \
                 WHERE device = (SELECT id FROM dict_device WHERE uuid = ?1)",
                [&device.as_bytes()[..]],
                |row| row.get(0),
            )
            .optional()
            .map_err(DbError::sqlite("reading the highest stored seq"))?
            .flatten();
        match max {
            None => Ok(0),
            Some(n) => {
                u64::try_from(n)
                    .ok()
                    .and_then(|n| n.checked_add(1))
                    .ok_or(DbError::OutOfRange {
                        what: "stored seq",
                        value: n.unsigned_abs(),
                    })
            }
        }
    }

    /// Writes one batch in one transaction: the events, the counter rows, and every session and
    /// dictionary row they need. Either all of it is committed and this returns `Ok`, or none of
    /// it is and this returns `Err` (the transaction rolls back when dropped).
    ///
    /// - Events use `INSERT OR IGNORE` on `(device, seq)`: a duplicate is skipped and counted.
    /// - Counter rows are deltas added to `ui_counter`.
    /// - An event whose payload cannot be encoded is skipped, counted in
    ///   [`BatchReport::events_rejected`] and logged: it cannot be fixed by retrying, and one bad
    ///   event must not block every later one.
    ///
    /// The in-memory dictionary caches are updated only after the commit, so a rolled-back batch
    /// leaves no stale entries.
    ///
    /// # Errors
    /// [`DbError`]; nothing was committed.
    pub fn write_batch(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
    ) -> Result<BatchReport, DbError> {
        let mut report = BatchReport::default();
        if events.is_empty() && counters.is_empty() {
            return Ok(report);
        }
        let tz = self.tz.get();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(DbError::sqlite("starting the batch transaction"))?;
        let mut resolver = Resolver::new(&mut self.caches);

        {
            let mut insert = tx
                .prepare_cached(INSERT_EVENT)
                .map_err(DbError::sqlite("preparing the event insert"))?;
            for event in events {
                let payload = match event.encode_payload() {
                    Ok(payload) => payload,
                    Err(error) => {
                        tracing::warn!(
                            kind = event.kind().name(),
                            seq = event.id.seq,
                            %error,
                            "signal event rejected: payload does not encode"
                        );
                        report.events_rejected += 1;
                        continue;
                    }
                };
                let device = resolver.resolve(&tx, Table::Device, event.id.device.as_bytes(), 0)?;
                let session = Self::session_row(&tx, &mut resolver, event, device, tz)?;
                let recording = event
                    .recording
                    .map(|r| resolver.resolve(&tx, Table::Recording, r.as_bytes(), 0))
                    .transpose()?;
                let source = event
                    .source
                    .map(|s| resolver.resolve(&tx, Table::Source, s.as_bytes(), 0))
                    .transpose()?;
                let changed = insert
                    .execute(params![
                        device,
                        to_i64("seq", event.id.seq)?,
                        event.ts.as_i64(),
                        session,
                        event.kind().discriminant(),
                        recording,
                        source,
                        event.pos_ds,
                        payload.as_ref().map(|p| p.as_bytes()),
                        event.ctx.map(|c| to_i64("ctx", c.0)).transpose()?,
                    ])
                    .map_err(DbError::sqlite("inserting an event"))?;
                if changed == 0 {
                    report.events_duplicate += 1;
                } else {
                    report.events_inserted += 1;
                }
            }
        }

        {
            let mut upsert = tx
                .prepare_cached(UPSERT_COUNTER)
                .map_err(DbError::sqlite("preparing the counter upsert"))?;
            for row in counters {
                let (table, kind_code) = match row.entity_kind {
                    EntityKind::Track => (Table::Recording, 0),
                    other => (Table::Entity, other.code()),
                };
                let entity = resolver.resolve(&tx, table, &row.entity, kind_code)?;
                upsert
                    .execute(params![
                        row.day.yyyymmdd(),
                        row.kind.discriminant(),
                        row.entity_kind.code(),
                        entity,
                        to_i64("counter count", row.count)?,
                        to_i64("counter total_ms", row.total_ms)?,
                        row.max_scrolled.raw(),
                    ])
                    .map_err(DbError::sqlite("adding a counter row"))?;
                report.counter_rows += 1;
            }
        }

        tx.commit()
            .map_err(DbError::sqlite("committing the batch"))?;
        resolver.publish();
        Ok(report)
    }

    /// The integer id of the event's session, inserting the `session` row the first time the
    /// session is seen. The row's start is the time embedded in the session's UUIDv7.
    fn session_row(
        conn: &Connection,
        resolver: &mut Resolver<'_>,
        event: &SignalEvent,
        device: i64,
        tz: TzOffsetMin,
    ) -> Result<i64, DbError> {
        let session: SessionId = event.session;
        if let Some(id) = resolver.known_session(session) {
            return Ok(id);
        }
        let mut insert = conn
            .prepare_cached(INSERT_SESSION)
            .map_err(DbError::sqlite("preparing the session insert"))?;
        insert
            .execute(params![device, session.created_at().as_i64(), tz.minutes()])
            .map_err(DbError::sqlite("inserting a session"))?;
        let id = conn.last_insert_rowid();
        resolver.new_session(session, id);
        Ok(id)
    }

    /// Dictionary and session entries currently held by the in-memory LRUs (never more than
    /// the configured capacities). Diagnostics and tests.
    #[must_use]
    pub fn cached_entries(&self) -> usize {
        self.caches.dict.len() + self.caches.sessions.len()
    }

    /// The typed error of the last failed [`SignalSink::write`], if the last write failed.
    #[must_use]
    pub fn last_error(&self) -> Option<&DbError> {
        self.last_error.as_ref()
    }

    /// What the last successful [`SignalSink::write`] did.
    #[must_use]
    pub fn last_report(&self) -> BatchReport {
        self.last_report
    }

    /// Current size of the write-ahead log file in bytes (0 when there is none).
    #[must_use]
    pub fn wal_bytes(&self) -> u64 {
        std::fs::metadata(&self.wal_path).map_or(0, |m| m.len())
    }

    /// Folds the WAL into the database file and truncates it, when it is due. Returns whether it
    /// checkpointed.
    ///
    /// Due means: the WAL reached 64 MiB (even with `idle == false`); or `idle` (the caller has
    /// nothing waiting to write) and either it reached 8 MiB or it is not empty and the last
    /// checkpoint is a minute old. At least 2 s apart, except for the 64 MiB case.
    ///
    /// The store turns SQLite's automatic checkpoint off because a checkpoint fsyncs: measured
    /// at 100 to 800 ms on Windows, it stalled the commit that triggered it and slowed the
    /// next batches. The writer calls this between batches instead. `TRUNCATE` (rather than
    /// `PASSIVE`) keeps the WAL file's length equal to its content, so the length is a usable
    /// trigger. The checkpoint never waits for a reader: if one still has the WAL open it
    /// reports busy and a later call tries again.
    ///
    /// # Errors
    /// [`DbError::Sqlite`]. The WAL stays valid; a later call tries again.
    pub fn checkpoint_if_due(&mut self, idle: bool) -> Result<bool, DbError> {
        let wal = self.wal_bytes();
        let since = self.last_checkpoint.elapsed();
        let due = wal >= WAL_HARD_LIMIT_BYTES
            || (idle
                && since >= WAL_CHECKPOINT_MIN_GAP
                && (wal >= WAL_SOFT_LIMIT_BYTES || (wal > 0 && since >= WAL_CHECKPOINT_INTERVAL)));
        if !due {
            return Ok(false);
        }
        self.last_checkpoint = Instant::now();
        self.conn
            .busy_timeout(Duration::ZERO)
            .map_err(DbError::sqlite("checkpointing the WAL"))?;
        let outcome: rusqlite::Result<i64> =
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0));
        let restored = self.conn.busy_timeout(connection::BUSY_TIMEOUT);
        let busy = outcome.map_err(DbError::sqlite("checkpointing the WAL"))?;
        restored.map_err(DbError::sqlite("restoring the busy timeout"))?;
        Ok(busy == 0)
    }

    /// Runs `PRAGMA optimize`, as `docs/rules/storage-budget.md` section 7 asks before closing.
    ///
    /// # Errors
    /// [`DbError::Sqlite`].
    pub fn optimize(&self) -> Result<(), DbError> {
        self.conn
            .execute_batch("PRAGMA optimize;")
            .map_err(DbError::sqlite("optimizing"))
    }
}

impl SignalSink for SignalStore {
    /// One call is one transaction ([`write_batch`](SignalStore::write_batch)). A failure keeps
    /// its typed [`DbError`] in [`last_error`](SignalStore::last_error) and crosses the bus
    /// boundary as the whole error chain in `SignalsError::Sink::reason`.
    fn write(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
    ) -> Result<(), SignalsError> {
        match self.write_batch(events, counters) {
            Ok(report) => {
                self.last_error = None;
                self.last_report = report;
                Ok(())
            }
            Err(error) => {
                let reason = error.chain();
                self.last_error = Some(error);
                Err(SignalsError::Sink { reason })
            }
        }
    }
}

fn to_i64(what: &'static str, value: u64) -> Result<i64, DbError> {
    i64::try_from(value).map_err(|_| DbError::OutOfRange { what, value })
}

/// Whether an event ends a play (or a session) and therefore forces an immediate flush: the
/// play record and the end of a listening decision should be on disk at once
/// (`docs/rules/database.md`, write path).
#[must_use]
pub fn ends_a_play(kind: SignalKind) -> bool {
    matches!(
        kind,
        SignalKind::Completed | SignalKind::Skipped | SignalKind::Paused | SignalKind::SessionEnded
    )
}
