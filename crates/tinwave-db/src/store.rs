//! `SignalStore`: the signals.db write path, one transaction per batch.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tinwave_core::{DeviceId, TzOffsetMin};
use tinwave_signals::{
    CounterRow, EntityKind, SignalEvent, SignalKind, SignalPayload, SignalSink, SignalsError,
};

use crate::connection;
use crate::dict::{DictCaches, Resolver, Table};
use crate::migrate::{migrate, Migration, MigrationReport};
use crate::play::{self, Ending, Outcome, PlayWrite};
use crate::signal_wipe;
use crate::DbError;

/// The migrations of `signals.db`, oldest first.
///
/// Append-only once merged to `main` (`docs/rules/database.md`): 0001 is frozen, a change goes
/// in the next number.
pub const SIGNALS_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/signals/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "signal_settings",
        sql: include_str!("../migrations/signals/0002_signal_settings.sql"),
    },
];

const INSERT_EVENT: &str = "INSERT OR IGNORE INTO signal_event \
    (device, seq, session, ts_off, kind, recording, source, pos_ds, payload, ctx) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)";

/// Counter rows are deltas: add the count and the milliseconds, keep the larger scroll depth.
const UPSERT_COUNTER: &str = "INSERT INTO ui_counter \
    (day, kind, entity_kind, entity, count, total_ms, max_scrolled) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
    ON CONFLICT (day, kind, entity_kind, entity) DO UPDATE SET \
    count = count + excluded.count, \
    total_ms = total_ms + excluded.total_ms, \
    max_scrolled = max(max_scrolled, excluded.max_scrolled)";

/// When the writer folds the WAL into the database file.
///
/// The store turns SQLite's automatic checkpoint off (see
/// [`SignalStore::checkpoint_if_due`]); this decides when the writer does it instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointPolicy {
    /// Checkpoint an idle store once the WAL reaches this size. Default 8 MiB.
    pub soft_limit_bytes: u64,
    /// Checkpoint at this size even if events are waiting. Bounds the disk the WAL can take (and
    /// the time a crash recovery replays) under an unbroken flood. Default 64 MiB.
    pub hard_limit_bytes: u64,
    /// An idle store checkpoints a non-empty WAL at least this often. With `synchronous=NORMAL` a
    /// commit is not fsynced, so this bounds what a power loss (not an application crash) can
    /// take: a typical listener commits a few pages every few minutes and would otherwise sit
    /// below the size limit for hours. Default 60 s.
    pub interval: Duration,
    /// Shortest time between two checkpoint attempts, whatever triggers them, so a reader that
    /// keeps the WAL busy does not make every commit retry (and fsync). Default 2 s.
    pub min_gap: Duration,
}

impl Default for CheckpointPolicy {
    fn default() -> Self {
        Self {
            soft_limit_bytes: 8 * 1024 * 1024,
            hard_limit_bytes: 64 * 1024 * 1024,
            interval: Duration::from_secs(60),
            min_gap: Duration::from_secs(2),
        }
    }
}

/// Sizes of the in-memory caches, the timezone and the checkpoint policy.
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
    /// When [`SignalStore::checkpoint_if_due`] checkpoints.
    pub checkpoint: CheckpointPolicy,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            dict_cache_capacity: 2_048,
            session_cache_capacity: 256,
            page_cache_kib: 256,
            tz_offset: TzOffsetMin::UTC,
            checkpoint: CheckpointPolicy::default(),
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
    /// Events dropped because they cannot be stored: the payload does not encode, or `seq` or
    /// `ctx` does not fit a SQLite integer (a producer bug; logged without values).
    pub events_rejected: usize,
    /// Counter rows added into `ui_counter`.
    pub counter_rows: usize,
    /// Counter rows dropped because a value does not fit a SQLite integer (logged).
    pub counter_rows_rejected: usize,
    /// `play` rows written for the plays that ended in this batch (`Completed` or `Skipped`).
    pub plays_written: usize,
    /// Outcomes stored in this batch that wrote no `play` row because no `PlayStarted` of the
    /// same recording precedes them on the device (logged).
    pub plays_unpaired: usize,
}

/// The signals.db writer connection: schema, dictionaries and the batch write.
///
/// One `SignalStore` owns the single writer connection of the file. It implements
/// [`SignalSink`] for the bus and keeps the typed [`DbError`] of the last failure
/// ([`last_error`](Self::last_error)); [`write_batch`](Self::write_batch) returns it directly.
pub struct SignalStore {
    conn: Connection,
    wal_path: PathBuf,
    checkpoint: CheckpointPolicy,
    last_checkpoint: Instant,
    caches: DictCaches,
    tz: SessionTz,
    last_error: Option<DbError>,
    last_report: BatchReport,
    /// The wipe epoch as of the last batch (`signal_wipe::wipe_epoch`): it changes only when a
    /// wipe of everything deleted the rows the caches point at.
    wipe_epoch: i32,
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
        let wipe_epoch = signal_wipe::wipe_epoch(&conn)?;
        let mut wal_path = path.as_os_str().to_owned();
        wal_path.push("-wal");
        Ok(Self {
            conn,
            wal_path: PathBuf::from(wal_path),
            checkpoint: config.checkpoint,
            last_checkpoint: Instant::now(),
            caches: DictCaches::new(config.dict_cache_capacity, config.session_cache_capacity),
            tz: SessionTz::new(config.tz_offset),
            last_error: None,
            last_report: BatchReport::default(),
            wipe_epoch,
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
    /// - Each newly stored `Completed` or `Skipped` writes the `play` row of its play in the same
    ///   transaction (see the `play` module docs for the pairing and what is written); a duplicate
    ///   writes none.
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
        // A wipe of everything deleted the dictionary and session rows since the last batch: the
        // cached integers may now point at nothing, or at another UUID. Only that wipe moves the
        // epoch (not every commit of another connection: the context hub and the settings write
        // all the time and keep the caches valid). Read inside the write lock, so no wipe can slip
        // in between the check and the batch.
        let epoch = signal_wipe::wipe_epoch(&tx)?;
        if epoch != self.wipe_epoch {
            self.caches.clear();
            self.wipe_epoch = epoch;
        }
        let mut resolver = Resolver::new(&mut self.caches);
        let mut outcomes: Vec<Outcome> = Vec::new();

        {
            let mut insert = tx
                .prepare_cached(INSERT_EVENT)
                .map_err(DbError::sqlite("preparing the event insert"))?;
            for event in events {
                let ctx = event.ctx.map(|c| fits(c.0));
                let (Some(seq), true) = (fits(event.id.seq), ctx != Some(None)) else {
                    reject_event(event, "seq or ctx does not fit a SQLite integer");
                    report.events_rejected += 1;
                    continue;
                };
                let ctx = ctx.flatten();
                let payload = match event.encode_payload() {
                    Ok(payload) => payload,
                    Err(error) => {
                        reject_event(event, variant_name(&error));
                        report.events_rejected += 1;
                        continue;
                    }
                };
                let device = resolver.resolve(&tx, Table::Device, event.id.device.as_bytes(), 0)?;
                let session = resolver.session(&tx, event.session, device, tz)?;
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
                        seq,
                        session.id,
                        event.ts.as_i64() - session.started_ts,
                        event.kind().discriminant(),
                        recording,
                        source,
                        event.pos_ds,
                        payload.as_ref().map(|p| p.as_bytes()),
                        ctx,
                    ])
                    .map_err(DbError::sqlite("inserting an event"))?;
                if changed == 0 {
                    report.events_duplicate += 1;
                } else {
                    report.events_inserted += 1;
                    // A play ends only with an outcome stored now: a duplicate (sync, retry)
                    // already wrote its row.
                    if let Some(ending) = ending(&event.payload) {
                        outcomes.push(Outcome {
                            device,
                            seq,
                            ts: event.ts.as_i64(),
                            recording,
                            ending,
                        });
                    }
                }
            }
        }

        // Every outcome of the batch is stored by now, so a start or an outcome that arrived in this
        // batch in any order is visible to the lookups.
        for outcome in &outcomes {
            match play::write_play(&tx, outcome)? {
                PlayWrite::Written => report.plays_written += 1,
                PlayWrite::Unpaired => {
                    tracing::warn!(
                        seq = outcome.seq,
                        "play outcome has no start of the same recording; no play record"
                    );
                    report.plays_unpaired += 1;
                }
            }
        }

        {
            let mut upsert = tx
                .prepare_cached(UPSERT_COUNTER)
                .map_err(DbError::sqlite("preparing the counter upsert"))?;
            for row in counters {
                let (Some(count), Some(total_ms)) = (fits(row.count), fits(row.total_ms)) else {
                    tracing::warn!(
                        kind = row.kind.name(),
                        "counter row rejected: a value does not fit a SQLite integer"
                    );
                    report.counter_rows_rejected += 1;
                    continue;
                };
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
                        count,
                        total_ms,
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

    /// The writer's connection, so that the integration tests can install a `trace_v2` hook and
    /// count the statements of a batch (`tests/common`, `store_statements_of`). Exists only with
    /// the `test-support` feature, which only this crate's own dev-dependencies enable: the default
    /// build exposes no raw connection. Not part of the API.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn connection_for_tests(&mut self) -> &mut Connection {
        &mut self.conn
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
    /// Due means, and never sooner than [`CheckpointPolicy::min_gap`] after the previous attempt:
    /// the WAL reached the hard limit (even with `idle == false`); or `idle` (the caller has
    /// nothing waiting to write) and either it reached the soft limit or it is not empty and the
    /// last checkpoint is [`CheckpointPolicy::interval`] old.
    ///
    /// The store turns SQLite's automatic checkpoint off because a checkpoint fsyncs: measured
    /// at 100 to 800 ms on Windows, it stalled the commit that triggered it and slowed the
    /// next batches. The writer calls this between batches, and also while idle (a commit is not
    /// fsynced, so an idle writer must still checkpoint on the interval). `TRUNCATE` (rather than
    /// `PASSIVE`) keeps the WAL file's length equal to its content, so the length is a usable
    /// trigger. The checkpoint never waits for a reader: if one still has the WAL open it
    /// reports busy and a later call tries again.
    ///
    /// # Errors
    /// [`DbError::Sqlite`]. The WAL stays valid; a later call tries again.
    pub fn checkpoint_if_due(&mut self, idle: bool) -> Result<bool, DbError> {
        let policy = self.checkpoint;
        let wal = self.wal_bytes();
        let since = self.last_checkpoint.elapsed();
        let due = since >= policy.min_gap
            && (wal >= policy.hard_limit_bytes
                || (idle
                    && (wal >= policy.soft_limit_bytes || (wal > 0 && since >= policy.interval))));
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

/// How an event ends a play, or `None` for every other kind.
fn ending(payload: &SignalPayload) -> Option<Ending> {
    match payload {
        SignalPayload::Completed(p) => Some(Ending::Completed {
            listened_ds: p.listened_ds.ds(),
        }),
        SignalPayload::Skipped(p) => Some(Ending::Skipped {
            fraction_raw: p.fraction.raw(),
        }),
        _ => None,
    }
}

/// `value` as a SQLite integer, or `None` above `i64::MAX`. Callers reject the event or row
/// instead of failing the batch: retrying cannot fix it, and one bad value must not wedge every
/// later one.
fn fits(value: u64) -> Option<i64> {
    i64::try_from(value).ok()
}

/// Logs a rejected event: its kind, `seq` and why, never its payload or any field value
/// (`docs/rules/rust.md`).
fn reject_event(event: &SignalEvent, why: &str) {
    tracing::warn!(
        kind = event.kind().name(),
        seq = event.id.seq,
        why,
        "signal event rejected"
    );
}

/// The name of an error variant, without any of its fields (they can hold payload values).
fn variant_name(error: &SignalsError) -> &'static str {
    match error {
        SignalsError::PayloadTooLong => "payload too long",
        SignalsError::NotFinite { .. } => "payload field not finite",
        SignalsError::InvalidValue { .. } => "payload field out of range",
        SignalsError::NotAnEvent(_) => "kind is not a raw event",
        _ => "payload does not encode",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::checksum;

    /// Migration 0001 is on `main` and in users' databases: its text is frozen. A change here
    /// means someone edited it; add a migration instead (`docs/rules/database.md`).
    #[test]
    fn migration_0001_is_frozen() {
        assert_eq!(
            checksum(SIGNALS_MIGRATIONS[0].sql),
            FROZEN_0001_CHECKSUM,
            "0001_init.sql was edited after it was merged to main"
        );
    }

    #[test]
    fn the_migrations_are_numbered_from_one() {
        for (i, m) in SIGNALS_MIGRATIONS.iter().enumerate() {
            assert_eq!(usize::try_from(m.version).unwrap(), i + 1, "{}", m.name);
        }
    }

    const FROZEN_0001_CHECKSUM: i64 = -2_299_945_054_729_597_038;
}
