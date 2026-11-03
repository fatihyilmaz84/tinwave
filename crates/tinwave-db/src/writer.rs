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
//! - the stage holds counter rows but no events and no derived values (they are already aggregated,
//!   nothing to wait for);
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
//! # Derived values
//!
//! The drain also hands over derived values (`RateSession`, stored into the `play` row of the play
//! they describe). They are staged with the events and committed in the same transaction, after the
//! play rows of the batch are written, so a play's value goes in **the transaction of its play-end
//! event** when the producer emitted it just before that event (the controller does; the bus merges
//! the two queues by `seq`).
//!
//! - **A value alone does not make the stage due.** It is emitted ahead of its outcome, so the stage
//!   waits for that outcome (which flushes at once) or for the flush interval, instead of committing
//!   the value into a transaction that holds no row for it.
//! - **A value that finds no row is held for one more commit**, then dropped and counted
//!   ([`WriterStats::derived_unmatched`]): its outcome may be a few events behind (the stage was cut
//!   at 256 events or at the interval between the two). A stopped play has no outcome, so its value
//!   is dropped by this rule.
//! - **Consent is asked again at the commit.** The stage can hold a value for up to one flush
//!   interval, so the writer asks the drain whether the kind is **still collected**
//!   ([`SignalDrain::enabled`]) just before it commits and drops the values of a kind that was
//!   switched off in the meantime ([`WriterStats::derived_withdrawn`]).
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
    CounterRow, DerivedValue, EntityKind, Fraction, LocalDay, SignalDrain, SignalEvent, SignalKind,
    SignalSink, SignalsError,
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
    /// Derived values stored into the `play` row they describe.
    pub derived_stored: u64,
    /// Derived values dropped because their play has no `play` row, after being held for one
    /// more commit (a stopped play, or one whose start was not recorded).
    pub derived_unmatched: u64,
    /// Derived values dropped because their kind was switched off while they waited in the stage.
    pub derived_withdrawn: u64,
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
    derived_stored: AtomicU64,
    derived_unmatched: AtomicU64,
    derived_withdrawn: AtomicU64,
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
            derived_stored: self.derived_stored.load(Ordering::Relaxed),
            derived_unmatched: self.derived_unmatched.load(Ordering::Relaxed),
            derived_withdrawn: self.derived_withdrawn.load(Ordering::Relaxed),
            failed_commits: self.failed_commits.load(Ordering::Relaxed),
            checkpoints: self.checkpoints.load(Ordering::Relaxed),
            events_lost: self.events_lost.load(Ordering::Relaxed),
        }
    }

    fn record_commit(&self, report: BatchReport, withdrawn: usize, unmatched: usize) {
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
        add(&self.derived_stored, report.derived_stored);
        add(&self.derived_unmatched, unmatched);
        add(&self.derived_withdrawn, withdrawn);
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
    derived: Vec<DerivedValue>,
    /// Values the last commit found no play row for: tried once more at the next commit. They are
    /// not "staged": they make nothing due and are not written alone.
    carried: Vec<DerivedValue>,
    counters: HashMap<CounterKey, Totals>,
    /// When the oldest staged event arrived.
    since: Option<Instant>,
    ends_play: bool,
}

impl Stage {
    fn is_empty(&self) -> bool {
        self.events.is_empty() && self.derived.is_empty() && self.counters.is_empty()
    }

    /// Empties what was staged. The `carried` values are the commit's to set.
    fn clear(&mut self) {
        self.events.clear();
        self.derived.clear();
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
        self.write_with_derived(events, counters, &[])
    }

    fn write_with_derived(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
        derived: &[DerivedValue],
    ) -> Result<(), SignalsError> {
        self.derived.extend_from_slice(derived);
        if !(events.is_empty() && derived.is_empty()) && self.since.is_none() {
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
        (self.stage.events.is_empty() && self.stage.derived.is_empty())
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
        // The stage may have held a value longer than the listener's consent lasted. The carried
        // values go first, so an index below `carried_len` is one of them.
        let consented = |value: &&DerivedValue| self.drain.enabled(value.kind());
        let mut derived: Vec<DerivedValue> = self
            .stage
            .carried
            .iter()
            .filter(consented)
            .copied()
            .collect();
        let carried_len = derived.len();
        derived.extend(self.stage.derived.iter().filter(consented).copied());
        let withdrawn = self.stage.carried.len() + self.stage.derived.len() - derived.len();
        if self.stage.events.is_empty() && rows.is_empty() && derived.is_empty() {
            // Only values of a kind that is off now: nothing to commit.
            self.shared.derived_withdrawn.fetch_add(
                u64::try_from(withdrawn).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
            self.stage.clear();
            self.stage.carried.clear();
            return Ok(());
        }
        let mut unmatched = Vec::new();
        let report = self.store.write_batch_collecting(
            &self.stage.events,
            &rows,
            &derived,
            &mut unmatched,
        )?;
        if self.retry_at.take().is_some() {
            tracing::info!("signals.db writes recovered");
        }
        // A value held once already is dropped now; a new one is held for the next commit.
        let dropped = unmatched.iter().filter(|&&i| i < carried_len).count();
        let held: Vec<DerivedValue> = unmatched
            .iter()
            .filter(|&&i| i >= carried_len)
            .map(|&i| derived[i])
            .collect();
        self.shared.record_commit(report, withdrawn, dropped);
        self.shared.set_last_error(None);
        self.stage.clear();
        self.stage.carried = held;
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
        let committed = self.try_commit();
        if committed.is_ok() {
            // Nothing follows, so a value still waiting for its play will not find it.
            self.shared.derived_unmatched.fetch_add(
                u64::try_from(self.stage.carried.len()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
            self.stage.carried.clear();
        }
        if let Err(error) = committed {
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

#[cfg(test)]
mod tests {
    use tinwave_core::{DeviceId, ManualClock, RecordingId, SessionId, UtcMillis};
    use tinwave_signals::{
        BusConfig, CompletedPayload, DerivedPayload, EmitOutcome, InputSource, PlayOrigin, PlayRef,
        PlayStartedPayload, PositionDs, Rate, RateSessionPayload, SignalBus, SignalCategory,
        SignalPayload, SignalSettings,
    };

    use super::*;

    const T0: i64 = 1_793_577_600_000;

    /// A worker over a temp database, nothing running on a thread: the tests drive `pull`,
    /// `due` and `try_commit` themselves and emit on the bus the way the controller does.
    struct Rig {
        worker: Worker,
        bus: SignalBus,
        settings: Arc<SignalSettings>,
        session: SessionId,
        recording: RecordingId,
        _dir: tempfile::TempDir,
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = SignalStore::open(&dir.path().join("signals.db")).unwrap();
            let at = UtcMillis::new(T0);
            let device = DeviceId::from_parts(at, [1; 10]).unwrap();
            let settings = Arc::new(SignalSettings::new());
            let (bus, drain) = SignalBus::channel(
                BusConfig::new(device),
                ManualClock::new(at),
                settings.clone(),
            )
            .unwrap();
            Self {
                worker: Worker::new(
                    store,
                    drain,
                    WriterConfig::default(),
                    Arc::new(Shared::default()),
                ),
                bus,
                settings,
                session: SessionId::from_parts(at, [2; 10]).unwrap(),
                recording: RecordingId::from_parts(at, [3; 10]).unwrap(),
                _dir: dir,
            }
        }

        fn emit(&self, payload: SignalPayload) {
            let outcome = self
                .bus
                .emit_with(payload.kind(), |stamp| SignalEvent {
                    id: stamp.id,
                    ts: stamp.ts,
                    session: self.session,
                    recording: Some(self.recording),
                    source: None,
                    pos_ds: None,
                    payload,
                    ctx: None,
                })
                .unwrap();
            assert_eq!(outcome, EmitOutcome::Queued);
        }

        fn start(&self) {
            self.emit(SignalPayload::PlayStarted(PlayStartedPayload {
                origin: PlayOrigin::Queue,
                input: InputSource::Auto,
            }));
        }

        fn complete(&self) {
            self.emit(SignalPayload::Completed(CompletedPayload {
                listened_ds: PositionDs::new(1_800).unwrap(),
                paused_ds: PositionDs::ZERO,
            }));
        }

        /// The rate of the play that started at `T0`, as the controller names it.
        fn rate(&self, raw: u8) {
            let outcome = self
                .bus
                .emit_derived(SignalKind::RateSession, |stamp| DerivedValue {
                    id: stamp.id,
                    ts: stamp.ts,
                    play: PlayRef {
                        session: self.session,
                        recording: self.recording,
                        started: UtcMillis::new(T0),
                    },
                    payload: DerivedPayload::RateSession(RateSessionPayload {
                        rate: Rate::from_raw(raw),
                    }),
                })
                .unwrap();
            assert_eq!(outcome, EmitOutcome::Queued);
        }

        /// Pulls everything the bus holds into the stage and commits it.
        fn flush(&mut self) {
            self.worker.pull(true);
            self.worker.try_commit().unwrap();
        }

        fn stats(&self) -> WriterStats {
            self.worker.shared.stats()
        }
    }

    /// The value is emitted just before its outcome, as the controller does: they are staged
    /// together, the stage is due because of the outcome, and one transaction writes the play
    /// row and the value in it.
    #[test]
    fn a_value_goes_into_the_transaction_of_the_outcome_that_follows_it() {
        let mut rig = Rig::new();
        rig.start();
        rig.flush();
        rig.rate(75);
        rig.complete();
        rig.worker.pull(true);
        assert!(rig.worker.due(Instant::now()), "the outcome ends a play");
        rig.worker.try_commit().unwrap();
        let stats = rig.stats();
        assert_eq!(stats.batches_committed, 2, "the start, then the outcome");
        assert_eq!(
            (
                stats.plays_written,
                stats.derived_stored,
                stats.derived_unmatched
            ),
            (1, 1, 0),
            "one transaction made the row and filled it: {stats:?}"
        );
        assert!(rig.worker.stage.carried.is_empty());
    }

    /// A value is emitted ahead of its outcome, so it must not make the stage commit on its own:
    /// that transaction would hold no row for it.
    #[test]
    fn a_value_alone_does_not_make_the_stage_due_but_the_interval_does() {
        let mut rig = Rig::new();
        rig.start();
        rig.flush();
        rig.rate(75);
        rig.worker.pull(true);
        assert_eq!(rig.worker.stage.derived.len(), 1);
        let now = Instant::now();
        assert!(!rig.worker.due(now), "waits for the outcome");
        assert!(
            rig.worker
                .due(now + rig.worker.config.flush_interval + Duration::from_millis(1)),
            "but not for ever: a stopped play has no outcome"
        );
    }

    /// The stage was cut between a value and its outcome (256 events, or the interval): the value
    /// finds no row, is held for one more commit, and is stored when the outcome is committed.
    #[test]
    fn a_value_committed_before_its_outcome_is_held_and_stored_with_it() {
        let mut rig = Rig::new();
        rig.start();
        rig.flush();
        rig.rate(75);
        rig.flush();
        let stats = rig.stats();
        assert_eq!(
            (stats.derived_stored, stats.derived_unmatched),
            (0, 0),
            "not stored yet, and not given up on either"
        );
        assert_eq!(rig.worker.stage.carried.len(), 1);

        rig.complete();
        rig.flush();
        let stats = rig.stats();
        assert_eq!((stats.derived_stored, stats.derived_unmatched), (1, 0));
        assert!(rig.worker.stage.carried.is_empty());
    }

    /// A stopped play has a value and no outcome: it is held once, then dropped and counted.
    #[test]
    fn a_value_with_no_play_is_dropped_at_the_second_commit() {
        let mut rig = Rig::new();
        rig.start();
        rig.flush();
        rig.rate(75);
        rig.flush();
        assert_eq!(rig.worker.stage.carried.len(), 1, "held");
        // Any later commit: here a favorite of something else.
        rig.emit(SignalPayload::Favorited);
        rig.flush();
        let stats = rig.stats();
        assert_eq!((stats.derived_stored, stats.derived_unmatched), (0, 1));
        assert!(rig.worker.stage.carried.is_empty());
    }

    /// Shutdown commits what it has and counts a value still waiting for its play: nothing follows.
    #[test]
    fn a_value_still_held_at_shutdown_is_counted_as_unmatched() {
        let mut rig = Rig::new();
        rig.start();
        rig.flush();
        rig.rate(75);
        rig.flush();
        let shared = rig.worker.shared.clone();
        rig.worker.finish().unwrap();
        assert_eq!(shared.stats().derived_unmatched, 1);
    }

    /// The stage can hold a value for a flush interval, so consent is asked again at the commit:
    /// a value whose kind was switched off while it waited is dropped, and nothing is committed
    /// for it.
    #[test]
    fn a_staged_value_whose_kind_was_switched_off_is_withdrawn_at_the_commit() {
        let mut rig = Rig::new();
        rig.rate(75);
        rig.worker.pull(true);
        assert_eq!(rig.worker.stage.derived.len(), 1, "the value is staged");
        rig.settings.set_category(SignalCategory::Speed, false);
        rig.worker.try_commit().unwrap();
        let stats = rig.stats();
        assert_eq!(stats.derived_withdrawn, 1);
        assert_eq!(
            (stats.derived_stored, stats.derived_unmatched),
            (0, 0),
            "the store never saw it"
        );
        assert_eq!(stats.batches_committed, 0, "no transaction for nothing");
        assert!(rig.worker.stage.is_empty());
    }

    /// A value that was held for the next commit is also asked about again.
    #[test]
    fn a_held_value_whose_kind_was_switched_off_is_withdrawn_too() {
        let mut rig = Rig::new();
        rig.start();
        rig.flush();
        rig.rate(75);
        rig.flush();
        assert_eq!(rig.worker.stage.carried.len(), 1);
        rig.settings.set_category(SignalCategory::Speed, false);
        rig.complete();
        rig.flush();
        let stats = rig.stats();
        assert_eq!((stats.derived_stored, stats.derived_withdrawn), (0, 1));
        assert_eq!(stats.plays_written, 1, "the play itself is still written");
    }
}
