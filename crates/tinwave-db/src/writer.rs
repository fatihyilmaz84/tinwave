//! `SignalWriter`: the thread that drains the bus into signals.db.
//!
//! # Flush policy
//!
//! The writer collects what the bus hands over in a small in-memory **stage** and commits the
//! stage with one [`SignalStore::write_batch`] (one transaction) when the first of these holds:
//!
//! - the stage holds [`WriterConfig::flush_events`] events (256);
//! - the oldest staged event is [`WriterConfig::flush_interval`] old (2 s);
//! - a staged event ends a play ([`ends_a_play`]): the play record must be on disk at once;
//! - the stage holds counter rows but no events (they are already aggregated, nothing to wait for);
//! - the writer is shutting down, or every producer handle is gone.
//!
//! The crash window is therefore one flush interval of events, as `docs/rules/database.md`
//! allows.
//!
//! # Why a stage, and how it meets the sink contract
//!
//! [`SignalDrain::pump`] hands over whatever is queued the moment it is called, and
//! [`SignalDrain::wait`] returns as soon as one event arrives. Pumping the store directly would
//! make one transaction per event. So the drain is pumped into the stage (an infallible
//! in-memory [`SignalSink`]), and the stage is committed to the store with the policy above. The
//! store's own contract (all or nothing, `Err` only when nothing was committed) is what makes
//! the retry below correct: the stage is kept as it is and offered again, events dedupe on
//! `(device, seq)`, and counter rows were never added.
//!
//! # Failure
//!
//! When a commit fails the writer logs it, keeps the stage, stops pulling from the bus (so the
//! bus's queues apply their drop policy and memory stays bounded) and retries every
//! [`WriterConfig::flush_interval`]: the same seconds-long interval `SignalDrain::wait` uses as
//! its retry backoff. The typed error is kept ([`SignalWriter::take_last_error`]).
//!
//! # Checkpoints
//!
//! A commit is not fsynced (`synchronous=NORMAL`), so committed events sit in the WAL until a
//! checkpoint. The writer asks the store to checkpoint ([`SignalStore::checkpoint_if_due`]) after
//! every flush and also when it is idle, so a play end flushed a minute ago is made durable even
//! if nothing else is ever written. The thresholds are [`CheckpointPolicy`](crate::CheckpointPolicy).
//!
//! # Shutdown
//!
//! [`SignalWriter::shutdown`] drains everything the bus has accepted, commits it, runs
//! `PRAGMA optimize` and joins the thread. The thread notices the request within
//! [`WriterConfig::stop_poll`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tinwave_core::TzOffsetMin;
use tinwave_signals::{
    CounterRow, EntityKind, Fraction, LocalDay, SignalDrain, SignalEvent, SignalKind, SignalSink,
    SignalsError,
};

use crate::store::{ends_a_play, BatchReport};
use crate::{DbError, SessionTz, SignalStore};

/// Timing and size of the writer's flushes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterConfig {
    /// Longest an event waits in memory, and the interval between retries after a failed commit.
    pub flush_interval: Duration,
    /// Commit as soon as this many events are staged.
    pub flush_events: usize,
    /// How often an idle writer looks at the shutdown flag. Waking is cheap (no IO); this only
    /// bounds how long [`SignalWriter::shutdown`] can take.
    pub stop_poll: Duration,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            flush_interval: Duration::from_secs(2),
            flush_events: 256,
            stop_poll: Duration::from_millis(250),
        }
    }
}

/// Counters the writer keeps for the Diagnostics page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriterStats {
    /// Transactions committed.
    pub batches_committed: u64,
    /// Events inserted.
    pub events_inserted: u64,
    /// Events skipped as `(device, seq)` duplicates.
    pub events_duplicate: u64,
    /// Events dropped because they cannot be stored (payload does not encode, `seq` or `ctx` too big).
    pub events_rejected: u64,
    /// Counter rows dropped because a value does not fit a SQLite integer.
    pub counter_rows_rejected: u64,
    /// Counter rows added.
    pub counter_rows: u64,
    /// `play` rows written (one per `Completed` or `Skipped` stored with its `PlayStarted`).
    pub plays_written: u64,
    /// Outcomes stored without a `PlayStarted` to pair with: no `play` row.
    pub plays_unpaired: u64,
    /// Commits that failed (each is retried).
    pub failed_commits: u64,
    /// WAL checkpoints run between batches.
    pub checkpoints: u64,
    /// Events lost at shutdown because the last commit failed.
    pub events_lost: u64,
}

#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    batches_committed: AtomicU64,
    events_inserted: AtomicU64,
    events_duplicate: AtomicU64,
    events_rejected: AtomicU64,
    counter_rows_rejected: AtomicU64,
    counter_rows: AtomicU64,
    plays_written: AtomicU64,
    plays_unpaired: AtomicU64,
    failed_commits: AtomicU64,
    checkpoints: AtomicU64,
    events_lost: AtomicU64,
    last_error: Mutex<Option<DbError>>,
}

impl Shared {
    fn stats(&self) -> WriterStats {
        WriterStats {
            batches_committed: self.batches_committed.load(Ordering::Relaxed),
            events_inserted: self.events_inserted.load(Ordering::Relaxed),
            events_duplicate: self.events_duplicate.load(Ordering::Relaxed),
            events_rejected: self.events_rejected.load(Ordering::Relaxed),
            counter_rows_rejected: self.counter_rows_rejected.load(Ordering::Relaxed),
            counter_rows: self.counter_rows.load(Ordering::Relaxed),
            plays_written: self.plays_written.load(Ordering::Relaxed),
            plays_unpaired: self.plays_unpaired.load(Ordering::Relaxed),
            failed_commits: self.failed_commits.load(Ordering::Relaxed),
            checkpoints: self.checkpoints.load(Ordering::Relaxed),
            events_lost: self.events_lost.load(Ordering::Relaxed),
        }
    }

    fn record_commit(&self, report: BatchReport) {
        let add = |counter: &AtomicU64, n: usize| {
            counter.fetch_add(u64::try_from(n).unwrap_or(u64::MAX), Ordering::Relaxed);
        };
        self.batches_committed.fetch_add(1, Ordering::Relaxed);
        add(&self.events_inserted, report.events_inserted);
        add(&self.events_duplicate, report.events_duplicate);
        add(&self.events_rejected, report.events_rejected);
        add(&self.counter_rows_rejected, report.counter_rows_rejected);
        add(&self.counter_rows, report.counter_rows);
        add(&self.plays_written, report.plays_written);
        add(&self.plays_unpaired, report.plays_unpaired);
    }

    fn set_last_error(&self, error: Option<DbError>) {
        *self
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = error;
    }
}

/// The signals.db writer thread. Dropping it shuts it down (and waits for the final flush);
/// call [`shutdown`](Self::shutdown) to see whether that flush succeeded.
pub struct SignalWriter {
    shared: Arc<Shared>,
    tz: SessionTz,
    thread: Option<JoinHandle<Result<(), DbError>>>,
}

impl std::fmt::Debug for SignalWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalWriter")
            .field("stats", &self.shared.stats())
            .finish_non_exhaustive()
    }
}

impl SignalWriter {
    /// Starts the writer thread. It owns `store` and `drain` from now on.
    ///
    /// # Errors
    /// [`DbError::Thread`] if the thread cannot be started.
    pub fn spawn(
        store: SignalStore,
        drain: SignalDrain,
        config: WriterConfig,
    ) -> Result<Self, DbError> {
        let shared = Arc::new(Shared::default());
        let tz = store.session_tz();
        let worker = Worker::new(store, drain, config, shared.clone());
        let thread = std::thread::Builder::new()
            .name("tinwave-signal-writer".to_owned())
            .spawn(move || worker.run())
            .map_err(DbError::Thread)?;
        Ok(Self {
            shared,
            tz,
            thread: Some(thread),
        })
    }

    /// Sets the timezone offset recorded on sessions first seen from now on. Call it together
    /// with `SignalBus::set_tz_offset` when a session starts.
    pub fn set_tz_offset(&self, tz: TzOffsetMin) {
        self.tz.set(tz);
    }

    /// The writer's counters.
    #[must_use]
    pub fn stats(&self) -> WriterStats {
        self.shared.stats()
    }

    /// The typed error of the most recent failed commit, if no commit has succeeded since.
    /// Returns it once.
    #[must_use]
    pub fn take_last_error(&self) -> Option<DbError> {
        self.shared
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Flushes everything the bus has accepted, stops the thread and returns its final counters.
    ///
    /// # Errors
    /// The error of the final commit if the database refused it (the events still staged are
    /// lost and counted in [`WriterStats::events_lost`]), or [`DbError::WriterPanicked`].
    pub fn shutdown(mut self) -> Result<WriterStats, DbError> {
        self.stop_and_join().map(|()| self.shared.stats())
    }

    fn stop_and_join(&mut self) -> Result<(), DbError> {
        self.shared.stop.store(true, Ordering::Release);
        match self.thread.take() {
            Some(thread) => thread.join().unwrap_or(Err(DbError::WriterPanicked)),
            None => Ok(()),
        }
    }
}

impl Drop for SignalWriter {
    fn drop(&mut self) {
        if let Err(error) = self.stop_and_join() {
            tracing::error!(error = %error.chain(), "signal writer stopped with an error");
        }
    }
}

type CounterKey = (LocalDay, SignalKind, EntityKind, [u8; 16]);

#[derive(Clone, Copy)]
struct Totals {
    count: u64,
    total_ms: u64,
    max_scrolled: u8,
}

/// Everything pulled from the bus and not yet committed.
#[derive(Default)]
struct Stage {
    events: Vec<SignalEvent>,
    counters: HashMap<CounterKey, Totals>,
    /// When the oldest staged event arrived.
    since: Option<Instant>,
    ends_play: bool,
}

impl Stage {
    fn is_empty(&self) -> bool {
        self.events.is_empty() && self.counters.is_empty()
    }

    fn clear(&mut self) {
        self.events.clear();
        self.counters.clear();
        self.since = None;
        self.ends_play = false;
    }

    /// The staged counters as rows, in a stable order.
    fn counter_rows(&self) -> Vec<CounterRow> {
        let mut rows: Vec<CounterRow> = self
            .counters
            .iter()
            .map(|(&(day, kind, entity_kind, entity), t)| CounterRow {
                day,
                kind,
                entity_kind,
                entity,
                count: t.count,
                total_ms: t.total_ms,
                max_scrolled: Fraction::from_raw(t.max_scrolled),
            })
            .collect();
        rows.sort_unstable_by_key(|r| (r.day, r.kind, r.entity_kind.code(), r.entity));
        rows
    }
}

/// The in-memory sink the drain is pumped into. It cannot fail.
impl SignalSink for Stage {
    fn write(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
    ) -> Result<(), SignalsError> {
        if !events.is_empty() && self.since.is_none() {
            self.since = Some(Instant::now());
        }
        self.ends_play |= events.iter().any(|e| ends_a_play(e.kind()));
        self.events.extend_from_slice(events);
        for row in counters {
            let totals = self
                .counters
                .entry((row.day, row.kind, row.entity_kind, row.entity))
                .or_insert(Totals {
                    count: 0,
                    total_ms: 0,
                    max_scrolled: 0,
                });
            totals.count = totals.count.saturating_add(row.count);
            totals.total_ms = totals.total_ms.saturating_add(row.total_ms);
            totals.max_scrolled = totals.max_scrolled.max(row.max_scrolled.raw());
        }
        Ok(())
    }
}

struct Worker {
    store: SignalStore,
    drain: SignalDrain,
    config: WriterConfig,
    shared: Arc<Shared>,
    stage: Stage,
    /// `Some` while the last commit failed: when to try again.
    retry_at: Option<Instant>,
    /// The next time counters are collected even if no event woke the drain.
    next_pump: Instant,
    /// Every producer handle is gone and the queues are empty.
    closed: bool,
}

impl Worker {
    fn new(
        store: SignalStore,
        drain: SignalDrain,
        config: WriterConfig,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            store,
            drain,
            config,
            shared,
            stage: Stage::default(),
            retry_at: None,
            next_pump: Instant::now() + config.flush_interval,
            closed: false,
        }
    }

    fn run(mut self) -> Result<(), DbError> {
        while !self.shared.stop.load(Ordering::Acquire) && !self.closed {
            let now = Instant::now();
            if let Some(at) = self.retry_at {
                // The last commit failed: do not pull more, so the bus's queues keep applying
                // their drop policy, and try again once the retry interval has passed.
                if now < at {
                    std::thread::sleep((at - now).min(self.config.stop_poll));
                } else {
                    self.commit();
                }
                continue;
            }
            let woke = self.drain.wait(self.wait_timeout(now));
            if woke || Instant::now() >= self.next_pump {
                self.pull(false);
            }
            if self.due(Instant::now()) {
                self.commit();
                self.maintain();
            } else if !woke && self.stage.is_empty() {
                // Idle: nothing to write and nothing arrived. A commit is not fsynced, so a play
                // end flushed a minute ago still sits in the WAL until a checkpoint (power-loss
                // exposure); `maintain` is rate-limited by the policy's minimum gap.
                self.maintain();
            }
        }
        self.finish()
    }

    /// How long to block: until the stage is due or counters are next collected, but never
    /// longer than `stop_poll` so a shutdown request is seen promptly.
    fn wait_timeout(&self, now: Instant) -> Duration {
        let due_at = match self.stage.since {
            Some(since) => since + self.config.flush_interval,
            None => self.next_pump,
        };
        due_at
            .saturating_duration_since(now)
            .min(self.config.stop_poll)
    }

    /// Moves what the bus holds into the stage. With `all`, until the queues are empty;
    /// otherwise until the stage is large enough to commit.
    fn pull(&mut self, all: bool) {
        loop {
            match self.drain.pump(&mut self.stage) {
                Ok(report) => {
                    self.closed |= report.closed;
                    if !report.more || (!all && self.stage.events.len() >= self.config.flush_events)
                    {
                        break;
                    }
                }
                Err(error) => {
                    // `Stage::write` never fails, so this is unreachable; if it ever is not,
                    // say so instead of looping on it.
                    tracing::error!(%error, "staging signals failed");
                    break;
                }
            }
        }
        self.next_pump = Instant::now() + self.config.flush_interval;
    }

    fn due(&self, now: Instant) -> bool {
        if self.stage.is_empty() {
            return false;
        }
        self.stage.events.is_empty()
            || self.stage.events.len() >= self.config.flush_events
            || self.stage.ends_play
            || self
                .stage
                .since
                .is_some_and(|since| now.duration_since(since) >= self.config.flush_interval)
    }

    /// Commits the stage in one transaction; on success clears it.
    fn try_commit(&mut self) -> Result<(), DbError> {
        if self.stage.is_empty() {
            return Ok(());
        }
        let rows = self.stage.counter_rows();
        let report = self.store.write_batch(&self.stage.events, &rows)?;
        if self.retry_at.take().is_some() {
            tracing::info!("signals.db writes recovered");
        }
        self.shared.record_commit(report);
        self.shared.set_last_error(None);
        self.stage.clear();
        Ok(())
    }

    /// Commits the stage. On failure keeps it, remembers the typed error and schedules a retry.
    fn commit(&mut self) {
        if let Err(error) = self.try_commit() {
            self.shared.failed_commits.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                error = %error.chain(),
                staged_events = self.stage.events.len(),
                retry_in_ms = u64::try_from(self.config.flush_interval.as_millis())
                    .unwrap_or(u64::MAX),
                "signals.db commit failed; the batch is kept and retried"
            );
            self.shared.set_last_error(Some(error));
            self.retry_at = Some(Instant::now() + self.config.flush_interval);
        }
    }

    /// Between batches: fold the WAL into the database file when it has grown (see
    /// [`SignalStore::checkpoint_if_due`]). A failure is logged and retried after the next batch.
    fn maintain(&mut self) {
        if self.retry_at.is_some() {
            return;
        }
        let idle = self.drain.queued() == 0;
        match self.store.checkpoint_if_due(idle) {
            Ok(true) => {
                self.shared.checkpoints.fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(error = %error.chain(), "signals.db checkpoint failed; will retry");
            }
        }
    }

    /// Graceful shutdown: everything the bus accepted is pulled and committed.
    fn finish(mut self) -> Result<(), DbError> {
        self.pull(true);
        if let Err(error) = self.try_commit() {
            let lost = self.stage.events.len();
            self.shared.failed_commits.fetch_add(1, Ordering::Relaxed);
            self.shared
                .events_lost
                .fetch_add(u64::try_from(lost).unwrap_or(u64::MAX), Ordering::Relaxed);
            tracing::error!(
                error = %error.chain(),
                lost_events = lost,
                "signals.db commit failed at shutdown"
            );
            return Err(error);
        }
        self.store.optimize()
    }
}
