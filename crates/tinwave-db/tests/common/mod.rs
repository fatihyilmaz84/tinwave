//! Fixtures shared by the integration tests and the benches: deterministic ids (injected clock
//! and seeded RNG), a database in a temp dir, and a synthetic listener.
//!
//! Compiled into several test binaries, each using a different part.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tinwave_core::{
    DeviceId, IdGenerator, ManualClock, RecordingId, Rng, SeededRng, SessionId, SignalEventId,
    SourceId, UtcMillis,
};
use tinwave_signals::{
    CompletedPayload, ElapsedDs, GestureTime, InputSource, PausedPayload, PlayOrigin,
    PlayStartedPayload, PositionDs, QueueOrigin, QueuedPayload, ResumedPayload, Section,
    SeekPayload, SignalEvent, SignalPayload, SkipPayload, View, ViewOpenedPayload, Volume,
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
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
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

/// A synthetic heavy listener (storage-budget.md reference user): a 5 000-track library with a
/// skewed popularity, a few sessions a day, and per play about nine events of the mix a real
/// session produces (play start, volume gestures, seeks, pause/resume, queueing, views, and the
/// end of the play as completed or skipped).
pub struct Listener {
    dice: Dice,
    device: DeviceId,
    recordings: Vec<(RecordingId, SourceId)>,
    session: SessionId,
    ids: Ids,
    seq: u64,
    ts: i64,
    plays_in_session: u32,
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
        }
    }

    pub fn device(&self) -> DeviceId {
        self.device
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
        out.push(
            Ev {
                device: self.device,
                seq: self.seq,
                ts: self.ts,
                session: self.session,
                recording: rec.map(|r| r.0),
                source: rec.map(|r| r.1),
                pos_ds,
                payload,
            }
            .build(),
        );
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
            self.session = self.ids.session();
            self.plays_in_session = 0;
            self.ts += 3 * 3_600_000;
        }
        self.plays_in_session += 1;
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
