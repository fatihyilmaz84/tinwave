//! Play records (T-0029): one `play` row per play that ends, written in the transaction of the
//! batch that stores its outcome. Every test drives the real `SignalStore` / `SignalWriter` on a
//! SQLite file; failures are injected from a second connection, as in `signal_store.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{count, Ids, TempDb, T0};
use rusqlite::Connection;
use tinwave_core::{DeviceId, RecordingId, SessionId, SignalEventId, SourceId, UtcMillis};
use tinwave_db::{EndReason, PlayFlags, SignalStore, SignalWriter, WriterConfig};
use tinwave_signals::{
    BusConfig, CompletedPayload, ContextKey, ContextSnapshotId, Daypart, Fraction, InputSource,
    PausedPayload, PlayOrigin, PlayStartedPayload, PositionDs, QueueOrigin, ResumedPayload,
    Section, SeekPayload, SignalBus, SignalEvent, SignalKind, SignalPayload, SignalSettings,
    SkipPayload, WeekdayType,
};

fn start() -> SignalPayload {
    SignalPayload::PlayStarted(PlayStartedPayload {
        origin: PlayOrigin::Queue,
        input: InputSource::Auto,
    })
}

fn skip(fraction_raw: u8) -> SignalPayload {
    SignalPayload::Skipped(SkipPayload {
        fraction: Fraction::from_raw(fraction_raw),
        input: InputSource::Mouse,
        section: Section::UNKNOWN,
        queue_origin: QueueOrigin::Manual,
    })
}

fn completed(listened_ds: u32) -> SignalPayload {
    SignalPayload::Completed(CompletedPayload {
        listened_ds: PositionDs::new(listened_ds).unwrap(),
        paused_ds: PositionDs::ZERO,
    })
}

fn pause() -> SignalPayload {
    SignalPayload::Paused(PausedPayload {
        input: InputSource::Keyboard,
    })
}

fn resume(pause_ms: u64) -> SignalPayload {
    SignalPayload::Resumed(ResumedPayload {
        pause_ds: PositionDs::from_ms(pause_ms),
        input: InputSource::Keyboard,
    })
}

fn seek_back() -> SignalPayload {
    SignalPayload::SeekBack(SeekPayload {
        from_ds: PositionDs::new(900).unwrap(),
        to_ds: PositionDs::new(100).unwrap(),
        section_from: Section::UNKNOWN,
        section_to: Section::UNKNOWN,
        input: InputSource::Mouse,
    })
}

/// A store on a temp file, one device and one session; events are numbered in the order they are
/// made.
struct Rig {
    db: TempDb,
    store: SignalStore,
    ids: Ids,
    device: DeviceId,
    session: SessionId,
    source: SourceId,
    seq: u64,
}

impl Rig {
    fn new() -> Self {
        let db = TempDb::new();
        let store = SignalStore::open(&db.path).unwrap();
        let mut ids = Ids::new(29);
        let (device, session, source) = (ids.device(), ids.session(), ids.source());
        Self {
            db,
            store,
            ids,
            device,
            session,
            source,
            seq: 0,
        }
    }

    /// An event of this rig's device and session, `at_ms` after `T0`, with the next `seq`.
    fn ev(&mut self, at_ms: i64, rec: RecordingId, payload: SignalPayload) -> SignalEvent {
        let event = event(
            self.device,
            self.session,
            self.seq,
            T0 + at_ms,
            Some(rec),
            Some(self.source),
            payload,
        );
        self.seq += 1;
        event
    }

    fn write(&mut self, events: &[SignalEvent]) -> tinwave_db::BatchReport {
        self.store.write_batch(events, &[]).unwrap()
    }

    fn plays(&self) -> Vec<Row> {
        plays(&self.db.reader())
    }
}

fn event(
    device: DeviceId,
    session: SessionId,
    seq: u64,
    ts: i64,
    rec: Option<RecordingId>,
    source: Option<SourceId>,
    payload: SignalPayload,
) -> SignalEvent {
    SignalEvent {
        id: SignalEventId::new(device, seq),
        ts: UtcMillis::new(ts),
        session,
        recording: rec,
        source,
        pos_ds: None,
        payload,
        ctx: None,
    }
}

fn with_ctx(mut event: SignalEvent, ctx: u64) -> SignalEvent {
    event.ctx = Some(ContextSnapshotId(ctx));
    event
}

/// A `play` row, with the dictionary ids resolved back to the UUIDs the test knows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    device: [u8; 16],
    session: [u8; 16],
    recording: [u8; 16],
    source: Option<[u8; 16]>,
    started_ts: i64,
    listened_ds: i64,
    end_reason: u8,
    end_frac: u8,
    flags: u16,
    peak_section: Option<i64>,
    context_key: u32,
}

fn plays(conn: &Connection) -> Vec<Row> {
    let uuid = |b: Vec<u8>| -> [u8; 16] { b.try_into().unwrap() };
    let mut stmt = conn
        .prepare(
            "SELECT d.uuid, s.uuid, r.uuid, src.uuid, p.started_ts, p.listened_ds, p.end_reason, \
                    p.end_frac, p.flags, p.peak_section, p.context_key \
             FROM play p \
             JOIN dict_device d ON d.id = p.device \
             JOIN session s ON s.id = p.session \
             JOIN dict_recording r ON r.id = p.recording \
             LEFT JOIN dict_source src ON src.id = p.source \
             ORDER BY p.id",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok(Row {
            device: uuid(row.get(0)?),
            session: uuid(row.get(1)?),
            recording: uuid(row.get(2)?),
            source: row.get::<_, Option<Vec<u8>>>(3)?.map(uuid),
            started_ts: row.get(4)?,
            listened_ds: row.get(5)?,
            end_reason: row.get(6)?,
            end_frac: row.get(7)?,
            flags: row.get(8)?,
            peak_section: row.get(9)?,
            context_key: row.get(10)?,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn add_snapshot(conn: &Connection, id: i64, key: i64) {
    conn.execute(
        "INSERT INTO context_snapshot (id, hash, key, fields) VALUES (?1, ?2, ?3, x'00')",
        rusqlite::params![id, id * 7919, key],
    )
    .unwrap();
}

// ---------------------------------------------------------------- one row per play end

/// Completed and the three skip kinds each write exactly one row, with the fields derived from the
/// play's events.
#[test]
fn each_play_end_writes_exactly_one_row() {
    let mut rig = Rig::new();
    let (a, b, c, d) = (
        rig.ids.recording(),
        rig.ids.recording(),
        rig.ids.recording(),
        rig.ids.recording(),
    );
    let events = [
        // Completed: 3 min 50 s listened.
        rig.ev(1_000, a, start()),
        rig.ev(231_000, a, completed(2_300)),
        // Skipped after 4 s, 5 % in: early.
        rig.ev(240_000, b, start()),
        rig.ev(244_000, b, skip(13)),
        // Skipped after 60 s, 40 % in: mid.
        rig.ev(250_000, c, start()),
        rig.ev(310_000, c, skip(102)),
        // Skipped after 170 s, 90 % in: late.
        rig.ev(320_000, d, start()),
        rig.ev(490_000, d, skip(230)),
    ];
    let report = rig.write(&events);
    assert_eq!(report.plays_written, 4);
    assert_eq!(report.plays_unpaired, 0);

    let rows = rig.plays();
    assert_eq!(rows.len(), 4, "one row per play end, not per event");
    let expect = |rec: RecordingId, started: i64, listened: i64, reason: EndReason, frac: u8| Row {
        device: *rig.device.as_bytes(),
        session: *rig.session.as_bytes(),
        recording: *rec.as_bytes(),
        source: Some(*rig.source.as_bytes()),
        started_ts: T0 + started,
        listened_ds: listened,
        end_reason: reason.code(),
        end_frac: frac,
        flags: 0,
        peak_section: None,
        context_key: 0,
    };
    assert_eq!(
        rows,
        [
            expect(a, 1_000, 2_300, EndReason::Completed, 255),
            expect(b, 240_000, 40, EndReason::SkippedEarly, 13),
            expect(c, 250_000, 600, EndReason::SkippedMid, 102),
            expect(d, 320_000, 1_700, EndReason::SkippedLate, 230),
        ]
    );
    assert_eq!(
        count(&rig.db.reader(), "SELECT count(*) FROM play"),
        4,
        "the table holds exactly the four rows"
    );
}

/// The row is part of the outcome's transaction: when the batch fails the row is rolled back with
/// the events, and the retry writes it once. A batch sent again writes nothing.
#[test]
fn the_play_row_is_written_in_the_transaction_of_its_outcome() {
    let mut rig = Rig::new();
    let a = rig.ids.recording();
    let events = [
        rig.ev(1_000, a, start()),
        rig.ev(60_000, a, skip(100)),
        rig.ev(61_000, a, SignalPayload::Favorited),
    ];

    // The play row is inserted after every event of the batch; making it fail proves that the
    // events and the row share one transaction.
    let reader = rig.db.reader();
    reader
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE INSERT ON play \
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    let error = rig.store.write_batch(&events, &[]).unwrap_err();
    assert!(
        error.chain().contains("injected failure"),
        "{}",
        error.chain()
    );
    assert_eq!(
        count(&reader, "SELECT count(*) FROM signal_event"),
        0,
        "a failing play row takes the batch's events with it"
    );
    assert_eq!(count(&reader, "SELECT count(*) FROM play"), 0);

    reader
        .execute_batch("DROP TRIGGER inject_failure;")
        .unwrap();
    let report = rig.store.write_batch(&events, &[]).unwrap();
    assert_eq!((report.events_inserted, report.plays_written), (3, 1));
    assert_eq!(count(&reader, "SELECT count(*) FROM play"), 1);

    // Sync dedupe / a re-sent batch: the events are duplicates, so no second row.
    let again = rig.store.write_batch(&events, &[]).unwrap();
    assert_eq!(
        (
            again.events_duplicate,
            again.plays_written,
            again.plays_unpaired
        ),
        (3, 0, 0)
    );
    assert_eq!(count(&reader, "SELECT count(*) FROM play"), 1);
}

// ---------------------------------------------------------------- plays that write no row

/// A track that fails writes no row: it leaves no outcome. The next play's row is its own.
#[test]
fn a_failed_track_writes_no_row() {
    let mut rig = Rig::new();
    let (failed, ok) = (rig.ids.recording(), rig.ids.recording());
    // `TrackLeft { Failed }` produces no signal: the failed play is a bare `PlayStarted`.
    let events = [
        rig.ev(1_000, failed, start()),
        rig.ev(5_000, ok, start()),
        rig.ev(65_000, ok, skip(100)),
    ];
    let report = rig.write(&events);
    assert_eq!((report.plays_written, report.plays_unpaired), (1, 0));
    let rows = rig.plays();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].recording, *ok.as_bytes());
    assert_eq!(rows[0].started_ts, T0 + 5_000, "paired with its own start");
    // The failed play's start stays in the hot tier.
    assert_eq!(
        count(
            &rig.db.reader(),
            "SELECT count(*) FROM signal_event WHERE kind = 53"
        ),
        2
    );
}

/// An outcome with no start of the same recording writes no row and says so: a row needs the
/// start's time and recording, and a guess would be a wrong listen.
#[test]
fn an_outcome_without_its_start_writes_no_row() {
    let mut rig = Rig::new();
    let (a, b, c) = (
        rig.ids.recording(),
        rig.ids.recording(),
        rig.ids.recording(),
    );
    let events = [
        // No start at all.
        rig.ev(1_000, a, skip(100)),
        // A start of another recording does not pair.
        rig.ev(2_000, b, start()),
        rig.ev(3_000, a, completed(10)),
        // A start is used once: the second outcome of the same play finds an outcome before it.
        rig.ev(4_000, c, start()),
        rig.ev(5_000, c, completed(20)),
        rig.ev(6_000, c, skip(50)),
    ];
    let report = rig.write(&events);
    assert_eq!((report.plays_written, report.plays_unpaired), (1, 3));
    let rows = rig.plays();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].recording, *c.as_bytes());
    assert_eq!(rows[0].end_reason, EndReason::Completed.code());
}

// ---------------------------------------------------------------- pairing (P-0111)

/// A play whose outcome lands in a later session (paused past 30 minutes, then skipped in the next
/// session) pairs per device by `seq`, in batches cut anywhere. The row belongs to the session
/// the play began in, and the time paused is not listened.
#[test]
fn an_outcome_in_a_later_session_pairs_by_device_not_session() {
    let mut rig = Rig::new();
    let (a, b) = (rig.ids.recording(), rig.ids.recording());
    let first_session = rig.session;
    // The second session opens 45 minutes later; its id carries that start time.
    rig.ids.clock.set(UtcMillis::new(T0 + 45 * 60_000));
    let second_session = rig.ids.session();
    assert_ne!(first_session, second_session);

    let (device, source) = (rig.device, rig.source);
    let ev = |seq: u64, ts: i64, session: SessionId, rec: RecordingId, p: SignalPayload| {
        event(device, session, seq, T0 + ts, Some(rec), Some(source), p)
    };
    // Session 1: the play starts, is paused a minute in, and the 30-minute silence ends the session.
    let in_first = [
        ev(0, 1_000, first_session, a, start()),
        ev(1, 61_000, first_session, a, pause()),
    ];
    // Session 2: the listener presses Next: the old play is skipped (in the new session), the
    // next one starts.
    let in_second = [
        ev(2, 45 * 60_000, second_session, a, skip(100)),
        ev(3, 45 * 60_000 + 500, second_session, b, start()),
        ev(4, 46 * 60_000, second_session, b, completed(300)),
    ];
    // Separate batches: the start is already committed when the outcome arrives.
    assert_eq!(rig.write(&in_first).plays_written, 0);
    let report = rig.write(&in_second);
    assert_eq!((report.plays_written, report.plays_unpaired), (2, 0));

    let rows = rig.plays();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].recording, *a.as_bytes());
    assert_eq!(
        rows[0].session,
        *first_session.as_bytes(),
        "the play belongs to the session it began in"
    );
    assert_eq!(rows[0].started_ts, T0 + 1_000);
    assert_eq!(
        rows[0].listened_ds, 600,
        "60 s played before the pause; the 44 minutes paused are not listening"
    );
    assert_eq!(rows[0].end_reason, EndReason::SkippedMid.code());
    assert_eq!(rows[1].recording, *b.as_bytes());
    assert_eq!(rows[1].session, *second_session.as_bytes());
}

/// Two devices write into one database (sync): each outcome pairs with the start of its own
/// device, however their `seq`s interleave.
#[test]
fn outcomes_pair_per_device() {
    let mut rig = Rig::new();
    let (x, y) = (rig.ids.recording(), rig.ids.recording());
    let other_device = rig.ids.device();
    let other_session = rig.ids.session();
    let (d1, s1, d2, s2, src) = (
        rig.device,
        rig.session,
        other_device,
        other_session,
        rig.source,
    );
    let ev = |d, s, seq: u64, ts: i64, rec, p| event(d, s, seq, T0 + ts, Some(rec), Some(src), p);
    // Device 1 plays x, device 2 plays y; their events interleave in time and in the batch.
    let events = [
        ev(d1, s1, 0, 1_000, x, start()),
        ev(d2, s2, 0, 1_500, y, start()),
        ev(d2, s2, 1, 40_000, y, completed(390)),
        ev(d1, s1, 1, 50_000, x, skip(60)),
    ];
    let report = rig.write(&events);
    assert_eq!((report.plays_written, report.plays_unpaired), (2, 0));
    let rows = rig.plays();
    let of = |device: DeviceId| {
        rows.iter()
            .find(|r| r.device == *device.as_bytes())
            .unwrap()
    };
    assert_eq!(of(d1).recording, *x.as_bytes());
    assert_eq!(of(d1).end_reason, EndReason::SkippedMid.code());
    assert_eq!(of(d1).listened_ds, 490);
    assert_eq!(of(d2).recording, *y.as_bytes());
    assert_eq!(of(d2).end_reason, EndReason::Completed.code());
    assert_eq!(of(d2).session, *s2.as_bytes());
}

/// The events of a batch may be handed over in any order (the bus does not promise `seq` order
/// across its queues): the outcome sorts out its start by `seq`, not by position in the slice.
#[test]
fn the_order_of_the_events_in_a_batch_does_not_matter() {
    let mut rig = Rig::new();
    let a = rig.ids.recording();
    let mut events = vec![
        rig.ev(1_000, a, start()),
        rig.ev(11_000, a, SignalPayload::Favorited),
        rig.ev(61_000, a, skip(100)),
    ];
    events.reverse();
    let report = rig.write(&events);
    assert_eq!(report.plays_written, 1);
    let rows = rig.plays();
    assert_eq!(rows[0].started_ts, T0 + 1_000);
    assert_eq!(rows[0].flags, PlayFlags::FAVORITED.bits());
}

// ---------------------------------------------------------------- listened time and flags

#[test]
fn pauses_are_not_listening_and_flags_come_from_the_plays_own_events() {
    let mut rig = Rig::new();
    let (a, other) = (rig.ids.recording(), rig.ids.recording());
    let events = [
        rig.ev(0, a, start()),
        // Paused 10 s..40 s, then paused again from 100 s until the skip at 130 s.
        rig.ev(10_000, a, pause()),
        rig.ev(40_000, a, resume(30_000)),
        rig.ev(70_000, a, seek_back()),
        rig.ev(100_000, a, pause()),
        // Favoriting another track while this one plays is not a flag of this play.
        rig.ev(110_000, other, SignalPayload::Favorited),
        rig.ev(130_000, a, skip(120)),
    ];
    rig.write(&events);
    let rows = rig.plays();
    assert_eq!(rows.len(), 1);
    // 130 s - 30 s paused - 30 s paused at the skip = 70 s.
    assert_eq!(rows[0].listened_ds, 700);
    assert_eq!(rows[0].flags, PlayFlags::SEEK_BACK.bits());

    // A favorite of this play's track sets its bit; a pause that never resumed before the next
    // play does not leak into it.
    let mut rig = Rig::new();
    let a = rig.ids.recording();
    let b = rig.ids.recording();
    let events = [
        rig.ev(0, a, start()),
        rig.ev(5_000, a, SignalPayload::Favorited),
        rig.ev(20_000, a, skip(30)),
        rig.ev(21_000, b, start()),
        rig.ev(81_000, b, skip(100)),
    ];
    rig.write(&events);
    let rows = rig.plays();
    assert_eq!(rows[0].flags, PlayFlags::FAVORITED.bits());
    assert_eq!(rows[0].listened_ds, 200);
    assert_eq!(rows[1].flags, 0);
    assert_eq!(rows[1].listened_ds, 600);
}

// ---------------------------------------------------------------- context key

/// The key is the one of the context snapshot the play's start points at. A play whose context
/// was never captured, or whose snapshot is missing or holds an unusable key, stores 0.
#[test]
fn the_context_key_comes_from_the_snapshot_of_the_start_and_defaults_to_unknown() {
    let mut rig = Rig::new();
    let key = ContextKey::UNKNOWN
        .with_daypart(Daypart::Evening)
        .with_weekday_type(WeekdayType::Weekend);
    let conn = rig.db.reader();
    add_snapshot(&conn, 1, i64::from(key.raw()));
    add_snapshot(&conn, 2, 0xDEAD_BEEF); // a version this build does not know
    add_snapshot(&conn, 3, -5); // not a u32 at all
    add_snapshot(&conn, 4, i64::from(ContextKey::UNKNOWN.raw()));
    let recs: Vec<RecordingId> = (0..6).map(|_| rig.ids.recording()).collect();
    let mut events = Vec::new();
    // (start ctx, outcome ctx): the outcome's snapshot never counts.
    let ctxs: [(Option<u64>, Option<u64>); 6] = [
        (Some(1), Some(4)), // valid key on the start
        (None, Some(1)),    // never captured at the start: unknown, whatever the outcome has
        (Some(2), None),    // unreadable version
        (Some(3), None),    // negative
        (Some(99), None),   // no such snapshot
        (Some(4), None),    // captured, nothing known
    ];
    for (i, (rec, (start_ctx, end_ctx))) in recs.iter().zip(ctxs).enumerate() {
        let at = i64::try_from(i).unwrap() * 100_000;
        let s = rig.ev(at, *rec, start());
        let e = rig.ev(at + 60_000, *rec, skip(100));
        events.push(start_ctx.map_or(s, |c| with_ctx(s, c)));
        events.push(end_ctx.map_or(e, |c| with_ctx(e, c)));
    }
    rig.write(&events);
    let keys: Vec<u32> = rig.plays().iter().map(|r| r.context_key).collect();
    assert_eq!(keys, [key.raw(), 0, 0, 0, 0, 0]);
    assert_ne!(key.raw(), 0);
    // What was stored unpacks to what the context engine would have packed.
    assert_eq!(ContextKey::from_raw(keys[0]), Ok(key));
}

// ---------------------------------------------------------------- the writer

/// Through the bus and the writer thread: a skip ends the play, the writer flushes at once and
/// the row is in the file with its events; the writer's counters say so.
#[test]
fn the_writer_commits_the_play_row_with_the_play_end() {
    let db = TempDb::new();
    let mut ids = Ids::new(31);
    let (device, session, rec, source) =
        (ids.device(), ids.session(), ids.recording(), ids.source());
    let store = SignalStore::open(&db.path).unwrap();
    let mut cfg = BusConfig::new(device);
    cfg.first_seq = store.first_seq(device).unwrap();
    let (bus, drain) =
        SignalBus::channel(cfg, ids.clock.clone(), Arc::new(SignalSettings::new())).unwrap();
    // A long interval: only the play end can flush in time.
    let config = WriterConfig {
        flush_interval: Duration::from_secs(600),
        flush_events: 1_000,
        stop_poll: Duration::from_millis(20),
    };
    let writer = SignalWriter::spawn(store, drain, config).unwrap();
    let make = |payload: SignalPayload| {
        move |stamp: tinwave_signals::EventStamp| SignalEvent {
            id: stamp.id,
            ts: stamp.ts,
            session,
            recording: Some(rec),
            source: Some(source),
            pos_ds: None,
            payload,
            ctx: None,
        }
    };
    let _ = bus
        .emit_with(SignalKind::PlayStarted, make(start()))
        .unwrap();
    ids.clock.advance(60_000);
    let _ = bus.emit_with(SignalKind::Skipped, make(skip(100))).unwrap();

    let reader = db.reader();
    let deadline = Instant::now() + Duration::from_secs(10);
    while count(&reader, "SELECT count(*) FROM play") == 0 {
        assert!(Instant::now() < deadline, "the play row was not flushed");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        count(&reader, "SELECT count(*) FROM signal_event"),
        2,
        "the row and both events were committed together"
    );
    let rows = plays(&reader);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].listened_ds, 600);
    assert_eq!(rows[0].end_reason, EndReason::SkippedMid.code());
    let stats = writer.shutdown().unwrap();
    assert_eq!((stats.plays_written, stats.plays_unpaired), (1, 0));
}
