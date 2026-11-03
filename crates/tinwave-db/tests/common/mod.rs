//! Fixtures shared by the integration tests and the benches: deterministic ids (injected clock
//! and seeded RNG), a database in a temp dir, and a synthetic listener.
//!
//! Compiled into several test binaries, each using a different part.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tinwave_core::{
    DeviceId, IdGenerator, ManualClock, RecordingId, Rng, SeededRng, SessionId, SignalEventId,
    SourceId, UtcMillis,
};
use tinwave_db::{BatchReport, DbError, SignalStore};
use tinwave_signals::{
    CompletedPayload, ContextSnapshotId, ElapsedDs, GestureTime, InputSource, PausedPayload,
    PlayOrigin, PlayStartedPayload, PositionDs, QueueOrigin, QueuedPayload, ResumedPayload,
    Section, SeekPayload, SignalEvent, SignalPayload, SkipPayload, View, ViewOpenedPayload, Volume,
    VolumePayload,
};

/// 2026-11-02T00:00:00Z.
pub const T0: i64 = 1_793_577_600_000;

/// A database file in a temp dir that lives as long as this value.
pub struct TempDb {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
}

impl TempDb {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signals.db");
        Self { dir, path }
    }

    /// A second connection, as a reader or to inject failures.
    pub fn reader(&self) -> Connection {
        reader(&self.path)
    }
}

pub fn reader(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn
}

pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Deterministic ids: a `ManualClock` and a `SeededRng` behind one `IdGenerator`.
pub struct Ids {
    pub clock: ManualClock,
    generator: IdGenerator,
}

impl Ids {
    pub fn new(seed: u64) -> Self {
        let clock = ManualClock::new(UtcMillis::new(T0));
        let generator = IdGenerator::new(clock.clone(), SeededRng::new(seed));
        Self { clock, generator }
    }

    pub fn device(&mut self) -> DeviceId {
        DeviceId::generate(&mut self.generator).unwrap()
    }

    pub fn recording(&mut self) -> RecordingId {
        RecordingId::generate(&mut self.generator).unwrap()
    }

    pub fn source(&mut self) -> SourceId {
        SourceId::generate(&mut self.generator).unwrap()
    }

    pub fn session(&mut self) -> SessionId {
        SessionId::generate(&mut self.generator).unwrap()
    }
}

/// A fully specified event.
pub struct Ev {
    pub device: DeviceId,
    pub seq: u64,
    pub ts: i64,
    pub session: SessionId,
    pub recording: Option<RecordingId>,
    pub source: Option<SourceId>,
    pub pos_ds: Option<u32>,
    pub payload: SignalPayload,
}

impl Ev {
    pub fn build(self) -> SignalEvent {
        SignalEvent {
            id: SignalEventId::new(self.device, self.seq),
            ts: UtcMillis::new(self.ts),
            session: self.session,
            recording: self.recording,
            source: self.source,
            pos_ds: self.pos_ds,
            payload: self.payload,
            ctx: None,
        }
    }
}

/// A unit-payload event (`Favorited`) at `seq`: cheap filler that is not a play end.
pub fn favorited(
    device: DeviceId,
    session: SessionId,
    recording: RecordingId,
    seq: u64,
) -> SignalEvent {
    Ev {
        device,
        seq,
        ts: T0 + i64::try_from(seq).unwrap() * 1_000,
        session,
        recording: Some(recording),
        source: None,
        pos_ds: None,
        payload: SignalPayload::Favorited,
    }
    .build()
}

/// A `Paused` event: a play end, which the writer flushes at once.
pub fn paused(
    device: DeviceId,
    session: SessionId,
    recording: RecordingId,
    seq: u64,
) -> SignalEvent {
    Ev {
        device,
        seq,
        ts: T0 + i64::try_from(seq).unwrap() * 1_000,
        session,
        recording: Some(recording),
        source: None,
        pos_ds: Some(1_000),
        payload: SignalPayload::Paused(PausedPayload {
            input: InputSource::Mouse,
        }),
    }
    .build()
}

/// A deterministic stream of 64-bit numbers for the listener model.
struct Dice(SeededRng);

impl Dice {
    fn next(&mut self) -> u64 {
        let mut bytes = [0u8; 8];
        self.0.fill_bytes(&mut bytes).unwrap();
        u64::from_le_bytes(bytes)
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// True with probability `percent` / 100.
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Tracks in the reference user's library (`docs/rules/testing.md`, `storage-budget.md`).
pub const REFERENCE_LIBRARY: usize = 50_000;

/// A synthetic heavy listener (storage-budget.md reference user): a library with a skewed
/// popularity (use [`REFERENCE_LIBRARY`]), a few sessions a day, and per play about 6.4 events
/// (measured, see `events_per_play`) of the mix a real session produces: play start, volume
/// gestures, seeks, pause/resume, queueing, views, and the end of the play as completed or
/// skipped.
///
/// Compared with the volume table of `docs/guides/signal-catalog.md` section 9 (~450 events a
/// day for 120 plays, ~3.75 per play) this model is about 70 % heavier in events, so the bytes
/// per event it measures are conservative for payload (more volume gestures, whose payload is the
/// largest) and neutral for the key columns. Every event carries a `ctx` snapshot id: a new id
/// about every 8 events (`ContextHub` reuses the previous id while the quantized fields are
/// unchanged; the context guide targets ~60 distinct snapshots a day, i.e. one per ~7.5 events of
/// the catalog's day), so ids are small integers that repeat in runs, as in real data.
pub struct Listener {
    dice: Dice,
    device: DeviceId,
    recordings: Vec<(RecordingId, SourceId)>,
    session: SessionId,
    ids: Ids,
    seq: u64,
    ts: i64,
    plays_in_session: u32,
    ctx: u64,
    plays: u64,
    emitted: u64,
}

impl Listener {
    pub fn new(seed: u64, library: usize) -> Self {
        let mut ids = Ids::new(seed);
        let device = ids.device();
        let recordings = (0..library)
            .map(|_| (ids.recording(), ids.source()))
            .collect();
        let session = ids.session();
        Self {
            dice: Dice(SeededRng::new(seed ^ 0xA5A5)),
            device,
            recordings,
            session,
            ids,
            seq: 0,
            ts: T0,
            plays_in_session: 0,
            ctx: 1,
            plays: 0,
            emitted: 0,
        }
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// Events generated per play so far (the doc of [`Listener`] quotes this).
    pub fn events_per_play(&self) -> f64 {
        self.emitted as f64 / self.plays.max(1) as f64
    }

    fn pick_track(&mut self) -> (RecordingId, SourceId) {
        // Squaring a uniform number skews picks toward the front of the library: a few
        // favourites played often, a long tail played rarely.
        let n = self.recordings.len() as u64;
        let u = self.dice.below(10_000);
        let index = (u * u * n) / (10_000 * 10_000);
        self.recordings[usize::try_from(index).unwrap()]
    }

    fn push(
        &mut self,
        out: &mut Vec<SignalEvent>,
        rec: Option<(RecordingId, SourceId)>,
        pos_ds: Option<u32>,
        payload: SignalPayload,
    ) {
        self.ts += i64::try_from(1 + self.dice.below(45)).unwrap() * 1_000;
        // A new context snapshot about every 8 events.
        if self.dice.chance(12) {
            self.ctx += 1;
        }
        self.emitted += 1;
        let ctx = ContextSnapshotId(self.ctx);
        let mut event = Ev {
            device: self.device,
            seq: self.seq,
            ts: self.ts,
            session: self.session,
            recording: rec.map(|r| r.0),
            source: rec.map(|r| r.1),
            pos_ds,
            payload,
        }
        .build();
        event.ctx = Some(ctx);
        out.push(event);
        self.seq += 1;
    }

    fn volume(&mut self) -> SignalPayload {
        let from = u8::try_from(40 + self.dice.below(160)).unwrap();
        let to = u8::try_from(40 + self.dice.below(160)).unwrap();
        SignalPayload::VolumeChanged(VolumePayload {
            from: Volume::from_raw(from),
            to: Volume::from_raw(to),
            gesture: GestureTime::from_ms(u32::try_from(200 + self.dice.below(1_500)).unwrap()),
            input: InputSource::Mouse,
            section: Section::UNKNOWN,
            since_track_start: ElapsedDs::from_ds(u16::try_from(self.dice.below(2_400)).unwrap()),
            since_section_start: ElapsedDs::from_ds(u16::try_from(self.dice.below(600)).unwrap()),
        })
    }

    /// Appends one play (about nine events) to `out`.
    pub fn play(&mut self, out: &mut Vec<SignalEvent>) {
        // A new session after a long silence: a few per day.
        if self.plays_in_session >= 30 + u32::try_from(self.dice.below(20)).unwrap() {
            self.plays_in_session = 0;
            self.ts += 3 * 3_600_000;
            // The session id is a UUIDv7 stamped by the injected clock: it carries the session start.
            self.ids.clock.set(UtcMillis::new(self.ts));
            self.session = self.ids.session();
        }
        self.plays_in_session += 1;
        self.plays += 1;
        let track = self.pick_track();
        let origin = if self.dice.chance(30) {
            PlayOrigin::Radio
        } else {
            PlayOrigin::Queue
        };
        self.push(
            out,
            Some(track),
            Some(0),
            SignalPayload::PlayStarted(PlayStartedPayload {
                origin,
                input: InputSource::Auto,
            }),
        );
        let mut pos = 0u32;
        for _ in 0..self.dice.below(4) {
            pos += u32::try_from(50 + self.dice.below(600)).unwrap();
            let payload = self.volume();
            self.push(out, Some(track), Some(pos), payload);
        }
        for _ in 0..self.dice.below(3) {
            let from = pos;
            pos = u32::try_from(self.dice.below(2_000)).unwrap();
            let payload = SeekPayload {
                from_ds: PositionDs::new(from).unwrap(),
                to_ds: PositionDs::new(pos).unwrap(),
                section_from: Section::UNKNOWN,
                section_to: Section::UNKNOWN,
                input: InputSource::Mouse,
            };
            self.push(
                out,
                Some(track),
                Some(from),
                if pos > from {
                    SignalPayload::SeekForward(payload)
                } else {
                    SignalPayload::SeekBack(payload)
                },
            );
        }
        if self.dice.chance(40) {
            let pause_ds = PositionDs::new(u32::try_from(self.dice.below(6_000)).unwrap()).unwrap();
            self.push(
                out,
                Some(track),
                Some(pos),
                SignalPayload::Paused(PausedPayload {
                    input: InputSource::Keyboard,
                }),
            );
            self.push(
                out,
                Some(track),
                Some(pos),
                SignalPayload::Resumed(ResumedPayload {
                    pause_ds,
                    input: InputSource::Keyboard,
                }),
            );
        }
        if self.dice.chance(60) {
            self.push(
                out,
                Some(track),
                None,
                SignalPayload::QueuedLater(QueuedPayload {
                    from_view: View::Lyrics,
                }),
            );
        }
        if self.dice.chance(50) {
            self.push(
                out,
                None,
                None,
                SignalPayload::ViewOpened(ViewOpenedPayload { view: View::Lyrics }),
            );
        }
        if self.dice.chance(30) {
            let fraction = u8::try_from(self.dice.below(256)).unwrap();
            self.push(
                out,
                Some(track),
                Some(pos),
                SignalPayload::Skipped(SkipPayload {
                    fraction: tinwave_signals::Fraction::from_raw(fraction),
                    input: InputSource::Mouse,
                    section: Section::UNKNOWN,
                    queue_origin: QueueOrigin::Radio,
                }),
            );
        } else {
            let paused_ds = PositionDs::new(u32::try_from(self.dice.below(100)).unwrap()).unwrap();
            self.push(
                out,
                Some(track),
                Some(2_400),
                SignalPayload::Completed(CompletedPayload {
                    listened_ds: PositionDs::new(2_300).unwrap(),
                    paused_ds,
                }),
            );
        }
    }

    /// At least `n` events, in whole plays, with strictly increasing `seq`.
    pub fn events(&mut self, n: usize) -> Vec<SignalEvent> {
        let mut out = Vec::with_capacity(n + 16);
        while out.len() < n {
            self.play(&mut out);
        }
        out
    }
}

/// Makes one statement of `on` (for example `"UPDATE ON play"`) hold the write lock for a few
/// hundred milliseconds: a trigger that fires once, for the row with `id = 1`, and burns time in
/// a cross join of a 3 000-row helper table. A test that must see a wipe's lock then does not
/// depend on how fast the machine is: the lock is held for at least as long as the burn. The
/// table must have an `id` column.
pub fn burn_in_trigger(conn: &Connection, on: &str) {
    conn.execute_batch(&format!(
        "CREATE TABLE burn (x INTEGER NOT NULL);
         WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 3000)
           INSERT INTO burn SELECT x FROM c;
         CREATE TRIGGER burn_in_trigger BEFORE {on} WHEN OLD.id = 1
         BEGIN SELECT count(*) FROM burn a, burn b WHERE a.x + b.x = -1; END;"
    ))
    .unwrap();
}

/// The busy timeout of every `signals.db` connection (`connection::BUSY_TIMEOUT`, private to the
/// crate): a statement that waits for a lock longer than this fails with "database is locked".
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long after a wipe returned a contended batch may still take to be stored: the wipe's lock
/// is gone, so only SQLite's busy back-off (at most 100 ms a step) and the write itself are left.
/// Measured (about 480 contended runs: idle, 16 and 32 heavy processes at once on 16 threads): the batch
/// was stored **before** the wipe returned in every one of them (the wipe's post-commit checkpoint
/// does not hold the write lock against it), so the figure is 0 s. The bound leaves 5 s for a
/// scheduling stall and still fails on a lock that outlives the wipe.
pub const AFTER_WIPE_SLACK: Duration = Duration::from_secs(5);

/// Whether `error` is SQLite's "database is locked": the only failure a contended writer batch may
/// have, and one that retrying cures.
pub fn is_busy(error: &DbError) -> bool {
    matches!(
        std::error::Error::source(error).and_then(|s| s.downcast_ref::<rusqlite::Error>()),
        Some(rusqlite::Error::SqliteFailure(failure, _))
            if failure.code == rusqlite::ErrorCode::DatabaseBusy
    )
}

/// The end of a wipe: set by [`during_a_wipe`] the moment the wipe call returned.
pub type WipeEnd = OnceLock<Instant>;

/// How a contended batch got in.
#[derive(Debug, Clone, Copy)]
pub struct Contended {
    /// Calls of `write_batch` (more than one: the busy timeout ran out and the writer retried).
    pub attempts: u32,
    /// From the first call to the stored batch.
    pub waited: Duration,
    /// From the end of the wipe to the stored batch; zero when the batch was stored before it.
    pub after_wipe: Duration,
}

/// What the writer thread does when a commit fails with a busy database: keep the batch and offer
/// it again (`writer.rs`, "the batch is kept and retried"). Not a bare `write_batch`: that gives
/// up after the 5 s busy timeout, and under machine load a wipe's lock (a burn trigger, the
/// scans, the checkpoint) can legitimately last longer than that.
///
/// Retries while the wipe runs and for [`AFTER_WIPE_SLACK`] after `wipe_end` is set; a failure that
/// is not "database is locked" is returned at once. The returned error is the last one.
pub fn write_batch_when_unlocked(
    store: &mut SignalStore,
    events: &[SignalEvent],
    wipe_end: &WipeEnd,
) -> (Result<BatchReport, DbError>, Contended) {
    let started = Instant::now();
    let mut attempts = 0;
    loop {
        attempts += 1;
        let result = store.write_batch(events, &[]);
        let now = Instant::now();
        let contended = Contended {
            attempts,
            waited: now - started,
            after_wipe: wipe_end
                .get()
                .map_or(Duration::ZERO, |end| now.saturating_duration_since(*end)),
        };
        match &result {
            Err(error) if is_busy(error) => {
                if contended.after_wipe > AFTER_WIPE_SLACK {
                    return (result, contended);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => return (result, contended),
        }
    }
}

/// Runs `wipe` on a thread and calls `during` once a second connection sees the wipe holding the
/// write lock (a probe with no busy timeout fails on `BEGIN IMMEDIATE`). Returns both results
/// and whether the lock was seen; a test must assert it, so a fast machine that finishes the
/// wipe before the first probe cannot silently skip the contention. `during` gets the instant the
/// wipe returned, once it has.
pub fn during_a_wipe<W: Send, D>(
    db: &TempDb,
    wipe: impl FnOnce() -> W + Send,
    during: impl FnOnce(&WipeEnd) -> D,
) -> (W, D, bool) {
    let probe = db.reader();
    probe.busy_timeout(Duration::ZERO).unwrap();
    let wipe_end = WipeEnd::new();
    std::thread::scope(|scope| {
        let wipe = scope.spawn(|| {
            let result = wipe();
            wipe_end.set(Instant::now()).unwrap();
            result
        });
        let mut seen = false;
        while !wipe.is_finished() {
            if probe.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").is_err() {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let result = during(&wipe_end);
        (wipe.join().unwrap(), result, seen)
    })
}
