//! Derived values (T-0070): a `RateSession` goes into the `play` row of its play, quantized, in the
//! transaction of the batch that holds it, and the wipes clear it. Every test drives the real
//! `SignalStore` / `SignalWriter` / `SignalWiper` on a SQLite file.
//!
//! The helpers are written out here (not added to `common`) so that this file stands alone.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{count, Ids, TempDb, T0};
use rusqlite::Connection;
use tinwave_core::{
    DeviceId, ManualClock, RecordingId, SessionId, SignalEventId, SourceId, Tier, UtcMillis,
};
use tinwave_db::{
    migrate, EndReason, SignalStore, SignalWiper, SignalWriter, WriterConfig, SIGNALS_MIGRATIONS,
};
use tinwave_signals::{
    BusConfig, CompletedPayload, DerivedPayload, DerivedValue, EmitOutcome, InputSource,
    PlayOrigin, PlayRef, PlayStartedPayload, PositionDs, Rate, RateSessionPayload, SignalBus,
    SignalCategory, SignalEvent, SignalKind, SignalPayload, SignalSettings,
};

fn start() -> SignalPayload {
    SignalPayload::PlayStarted(PlayStartedPayload {
        origin: PlayOrigin::Queue,
        input: InputSource::Auto,
    })
}

fn completed(listened_ds: u32) -> SignalPayload {
    SignalPayload::Completed(CompletedPayload {
        listened_ds: PositionDs::new(listened_ds).unwrap(),
        paused_ds: PositionDs::ZERO,
    })
}

/// A store on a temp file with one device and session; events are numbered in the order they are
/// made.
struct Rig {
    db: TempDb,
    store: SignalStore,
    ids: Ids,
    device: DeviceId,
    session: SessionId,
    source: SourceId,
    seq: u64,
    /// The plays seeded made: recording and start, ms after T0.
    made: Vec<(RecordingId, i64)>,
}

impl Rig {
    fn new() -> Self {
        let db = TempDb::new();
        let store = SignalStore::open(&db.path).unwrap();
        let mut ids = Ids::new(70);
        let (device, session, source) = (ids.device(), ids.session(), ids.source());
        Self {
            db,
            store,
            ids,
            device,
            session,
            source,
            seq: 0,
            made: Vec::new(),
        }
    }

    fn ev_in(
        &mut self,
        session: SessionId,
        at_ms: i64,
        rec: RecordingId,
        payload: SignalPayload,
    ) -> SignalEvent {
        let event = SignalEvent {
            id: SignalEventId::new(self.device, self.seq),
            ts: UtcMillis::new(T0 + at_ms),
            session,
            recording: Some(rec),
            source: Some(self.source),
            pos_ds: None,
            payload,
            ctx: None,
        };
        self.seq += 1;
        event
    }

    fn ev(&mut self, at_ms: i64, rec: RecordingId, payload: SignalPayload) -> SignalEvent {
        self.ev_in(self.session, at_ms, rec, payload)
    }

    /// The start and the completion of one play of `rec`, begun `at_ms` after `T0`.
    fn play(&mut self, at_ms: i64, rec: RecordingId) -> [SignalEvent; 2] {
        [
            self.ev(at_ms, rec, start()),
            self.ev(at_ms + 180_000, rec, completed(1_800)),
        ]
    }

    /// A rate value for the play of `rec` that began `at_ms` after `T0` in `session`.
    fn rate_of(
        &mut self,
        session: SessionId,
        rec: RecordingId,
        at_ms: i64,
        raw: u8,
    ) -> DerivedValue {
        let value = DerivedValue {
            id: SignalEventId::new(self.device, self.seq),
            ts: UtcMillis::new(T0 + at_ms + 180_000),
            play: PlayRef {
                session,
                recording: rec,
                started: UtcMillis::new(T0 + at_ms),
            },
            payload: DerivedPayload::RateSession(RateSessionPayload {
                rate: Rate::from_raw(raw),
            }),
        };
        self.seq += 1;
        value
    }

    fn write(
        &mut self,
        events: &[SignalEvent],
        derived: &[DerivedValue],
    ) -> tinwave_db::BatchReport {
        self.store
            .write_batch_with_derived(events, &[], derived)
            .unwrap()
    }

    fn rates(&self) -> Vec<(i64, Option<i64>)> {
        rates(&self.db.reader())
    }
}

/// `(started_ts - T0, rate)` of every play, in write order.
fn rates(conn: &Connection) -> Vec<(i64, Option<i64>)> {
    let mut stmt = conn
        .prepare("SELECT started_ts - ?1, rate FROM play ORDER BY id")
        .unwrap();
    stmt.query_map([T0], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

// ---------------------------------------------------------------- storing

#[test]
fn a_rate_session_is_stored_into_the_play_row_of_its_play_after_the_outcome_was_committed() {
    let mut rig = Rig::new();
    let rec = rig.ids.recording();
    let events = rig.play(1_000, rec);
    let report = rig.write(&events, &[]);
    assert_eq!(report.plays_written, 1);
    assert_eq!(rig.rates(), [(1_000, None)], "not measured yet");

    let value = rig.rate_of(rig.session, rec, 1_000, 75);
    let report = rig.write(&[], &[value]);
    assert_eq!((report.derived_stored, report.derived_unmatched), (1, 0));
    assert_eq!(rig.rates(), [(1_000, Some(75))]);
}

#[test]
fn a_value_in_the_batch_of_its_outcome_finds_the_row_written_in_that_batch() {
    let mut rig = Rig::new();
    let rec = rig.ids.recording();
    let events = rig.play(1_000, rec);
    let value = rig.rate_of(rig.session, rec, 1_000, 60);
    let report = rig.write(&events, &[value]);
    assert_eq!(
        (report.plays_written, report.derived_stored),
        (1, 1),
        "the value is applied after the play rows of its own batch"
    );
    assert_eq!(rig.rates(), [(1_000, Some(60))]);
}

#[test]
fn a_value_is_stored_only_into_the_play_it_names() {
    let mut rig = Rig::new();
    let (a, b) = (rig.ids.recording(), rig.ids.recording());
    let other_session = rig.ids.session();
    let first = rig.play(1_000, a);
    let same_recording_later = rig.play(500_000, a);
    let other_recording = rig.play(1_000, b);
    // The same recording at the same time in another session is another play.
    let in_other_session = [
        rig.ev_in(other_session, 1_000, a, start()),
        rig.ev_in(other_session, 181_000, a, completed(1_800)),
    ];
    for events in [
        &first,
        &same_recording_later,
        &other_recording,
        &in_other_session,
    ] {
        rig.write(events, &[]);
    }
    assert_eq!(count(&rig.db.reader(), "SELECT count(*) FROM play"), 4);

    // Four plays share a recording, a start time or a session with the named one, none all three.
    let value = rig.rate_of(rig.session, a, 1_000, 90);
    let report = rig.write(&[], &[value]);
    assert_eq!(report.derived_stored, 1);
    assert_eq!(
        rig.rates(),
        [
            (1_000, Some(90)),
            (500_000, None),
            (1_000, None),
            (1_000, None)
        ],
        "only the play with that recording, session and start time"
    );
    let value = rig.rate_of(rig.session, a, 500_000, 91);
    rig.write(&[], &[value]);
    assert_eq!(
        rig.rates(),
        [
            (1_000, Some(90)),
            (500_000, Some(91)),
            (1_000, None),
            (1_000, None)
        ]
    );
}

#[test]
fn a_value_whose_play_has_no_row_is_dropped_and_creates_no_row_at_all() {
    let mut rig = Rig::new();
    let rec = rig.ids.recording();
    // A stopped play: its start was recorded, its outcome never was, so there is no row.
    let start_only = rig.ev(1_000, rec, start());
    rig.write(&[start_only], &[]);
    let stranger = rig.ids.recording();
    let unknown_session = rig.ids.session();
    let before = |conn: &Connection| {
        [
            "dict_recording",
            "dict_device",
            "dict_source",
            "session",
            "play",
        ]
        .map(|table| count(conn, &format!("SELECT count(*) FROM {table}")))
    };
    let tables_before = before(&rig.db.reader());

    let values = [
        rig.rate_of(rig.session, rec, 1_000, 60),
        rig.rate_of(rig.session, stranger, 1_000, 60),
        rig.rate_of(unknown_session, rec, 1_000, 60),
    ];
    let report = rig.write(&[], &values);
    assert_eq!((report.derived_stored, report.derived_unmatched), (0, 3));
    assert_eq!(
        before(&rig.db.reader()),
        tables_before,
        "nothing was created"
    );
    assert_eq!(rig.rates(), [] as [(i64, Option<i64>); 0]);
}

#[test]
fn storing_a_value_twice_leaves_the_same_row() {
    let mut rig = Rig::new();
    let rec = rig.ids.recording();
    let events = rig.play(1_000, rec);
    rig.write(&events, &[]);
    let value = rig.rate_of(rig.session, rec, 1_000, 80);
    rig.write(&[], &[value]);
    rig.write(&[], &[value]);
    assert_eq!(rig.rates(), [(1_000, Some(80))]);
}

#[test]
fn the_column_holds_hundredths_minus_fifty_and_a_play_never_measured_holds_null() {
    let mut rig = Rig::new();
    let (measured, unmeasured) = (rig.ids.recording(), rig.ids.recording());
    let a = rig.play(1_000, measured);
    let b = rig.play(300_000, unmeasured);
    rig.write(&a, &[]);
    rig.write(&b, &[]);
    // 1.25x is 125 hundredths, minus 50.
    let raw = Rate::from_rate(1.25).unwrap().raw();
    assert_eq!(raw, 75);
    let value = rig.rate_of(rig.session, measured, 1_000, raw);
    rig.write(&[], &[value]);
    assert_eq!(rig.rates(), [(1_000, Some(75)), (300_000, None)]);
    // An integer that fits one byte, however much of the range is used.
    let typeof_rate: String = rig
        .db
        .reader()
        .query_row(
            "SELECT typeof(rate) FROM play WHERE rate IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(typeof_rate, "integer");
}

#[test]
fn a_database_from_before_the_column_gains_it_and_keeps_its_plays() {
    let db = TempDb::new();
    {
        let mut conn = Connection::open(&db.path).unwrap();
        // What a database made by the build before this column has: the pragmas that a store insists on.
        conn.pragma_update(None, "page_size", 4096).unwrap();
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")
            .unwrap();
        migrate(&mut conn, &SIGNALS_MIGRATIONS[..2]).unwrap();
        conn.execute_batch(
            "INSERT INTO dict_device (id, uuid) VALUES (1, x'01'); \
             INSERT INTO dict_recording (id, uuid) VALUES (1, x'02'); \
             INSERT INTO session (id, uuid, device, started_ts, tz_offset_min) VALUES (1, x'03', 1, 5, 0); \
             INSERT INTO play (device, session, recording, started_ts, listened_ds, end_reason, \
                               end_frac, flags, peak_section, context_key) \
             VALUES (1, 1, 1, 5, 100, 0, 255, 0, NULL, 0);",
        )
        .unwrap();
    }
    let _store = SignalStore::open(&db.path).unwrap();
    let conn = db.reader();
    let row: (i64, Option<i64>) = conn
        .query_row("SELECT listened_ds, rate FROM play", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(row, (100, None));
}

// ---------------------------------------------------------------- the writer thread

/// A bus, a writer and the events of one play, built the way the controller builds them.
struct Live {
    db: TempDb,
    bus: SignalBus,
    writer: SignalWriter,
    settings: Arc<SignalSettings>,
    session: SessionId,
    recording: RecordingId,
    source: SourceId,
}

impl Live {
    fn new(config: WriterConfig) -> Self {
        let db = TempDb::new();
        let mut ids = Ids::new(71);
        let device = ids.device();
        let clock = ManualClock::new(UtcMillis::new(T0));
        let settings = Arc::new(SignalSettings::new());
        let store = SignalStore::open(&db.path).unwrap();
        let (bus, drain) =
            SignalBus::channel(BusConfig::new(device), clock, settings.clone()).unwrap();
        let writer = SignalWriter::spawn(store, drain, config).unwrap();
        Self {
            db,
            bus,
            writer,
            settings,
            session: ids.session(),
            recording: ids.recording(),
            source: ids.source(),
        }
    }

    fn emit(&self, kind: SignalKind, payload: SignalPayload) -> UtcMillis {
        let mut at = UtcMillis::new(0);
        let outcome = self
            .bus
            .emit_with(kind, |stamp| {
                at = stamp.ts;
                SignalEvent {
                    id: stamp.id,
                    ts: stamp.ts,
                    session: self.session,
                    recording: Some(self.recording),
                    source: Some(self.source),
                    pos_ds: None,
                    payload,
                    ctx: None,
                }
            })
            .unwrap();
        assert_eq!(outcome, EmitOutcome::Queued);
        at
    }

    fn emit_rate(&self, started: UtcMillis, raw: u8) -> EmitOutcome {
        self.bus
            .emit_derived(SignalKind::RateSession, |stamp| DerivedValue {
                id: stamp.id,
                ts: stamp.ts,
                play: PlayRef {
                    session: self.session,
                    recording: self.recording,
                    started,
                },
                payload: DerivedPayload::RateSession(RateSessionPayload {
                    rate: Rate::from_raw(raw),
                }),
            })
            .unwrap()
    }

    fn wait_for(&self, what: &str, done: impl Fn(&tinwave_db::WriterStats) -> bool) {
        let begun = Instant::now();
        while !done(&self.writer.stats()) {
            assert!(begun.elapsed() < Duration::from_secs(30), "never: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// The order the controller uses: the value just before the outcome. The outcome flushes at once
/// and takes the value with it, so the play row is made and filled by one transaction, whatever
/// the thread timing.
#[test]
fn the_writer_stores_a_value_emitted_just_before_the_outcome_in_its_transaction() {
    for _ in 0..20 {
        let live = Live::new(WriterConfig::default());
        let started = live.emit(SignalKind::PlayStarted, start());
        assert_eq!(live.emit_rate(started, 68), EmitOutcome::Queued);
        live.emit(SignalKind::Completed, completed(1_800));
        live.wait_for("the value is stored", |s| {
            s.derived_stored + s.derived_unmatched == 1
        });
        let stats = live.writer.stats();
        assert_eq!(
            (stats.derived_stored, stats.derived_unmatched),
            (1, 0),
            "{stats:?}"
        );
        assert_eq!(stats.plays_written, 1);
        assert_eq!(
            rates(&live.db.reader()),
            [(started.as_i64() - T0, Some(68))]
        );
    }
}

/// A producer that emits the value after the outcome is still served: the value waits for the
/// flush interval (it alone does not make the writer commit) and then finds the row.
#[test]
fn a_value_emitted_after_the_outcome_is_stored_at_the_next_flush() {
    let live = Live::new(WriterConfig {
        flush_interval: Duration::from_millis(50),
        ..WriterConfig::default()
    });
    let started = live.emit(SignalKind::PlayStarted, start());
    live.emit(SignalKind::Completed, completed(1_800));
    assert_eq!(live.emit_rate(started, 68), EmitOutcome::Queued);
    live.wait_for("the value is stored", |s| s.derived_stored == 1);
    assert_eq!(live.writer.stats().derived_unmatched, 0);
    assert_eq!(
        rates(&live.db.reader()),
        [(started.as_i64() - T0, Some(68))]
    );
}

/// A play that was stopped has a value and no outcome, so no record: the writer drops the value
/// and counts it once the flush interval has passed twice.
#[test]
fn a_value_of_a_play_that_has_no_outcome_is_dropped_and_counted() {
    let live = Live::new(WriterConfig {
        flush_interval: Duration::from_millis(50),
        ..WriterConfig::default()
    });
    let started = live.emit(SignalKind::PlayStarted, start());
    assert_eq!(live.emit_rate(started, 68), EmitOutcome::Queued);
    // The first flush finds no row and holds the value; any later commit lets the writer give up.
    live.wait_for("the first flush", |s| s.batches_committed >= 1);
    assert_eq!(
        live.writer.stats().derived_unmatched,
        0,
        "held, not dropped"
    );
    live.emit(SignalKind::Favorited, SignalPayload::Favorited);
    live.wait_for("the value is given up on", |s| s.derived_unmatched == 1);
    assert_eq!(live.writer.stats().derived_stored, 0);
    assert_eq!(count(&live.db.reader(), "SELECT count(*) FROM play"), 0);
}

#[test]
fn nothing_is_stored_with_the_speed_category_off() {
    let live = Live::new(WriterConfig::default());
    let started = live.emit(SignalKind::PlayStarted, start());
    live.emit(SignalKind::Completed, completed(1_800));
    live.settings.set_category(SignalCategory::Speed, false);
    assert_eq!(live.emit_rate(started, 68), EmitOutcome::Disabled);
    assert_eq!(live.bus.derived_queued_total(), 0);
    live.wait_for("the play is written", |s| s.plays_written == 1);
    let stats = live.writer.shutdown().unwrap();
    assert_eq!(stats.derived_stored, 0);
    let conn = live.db.reader();
    assert_eq!(rates(&conn), [(started.as_i64() - T0, None)]);
}

#[test]
fn nothing_is_stored_with_the_tier_of_its_input_off() {
    let live = Live::new(WriterConfig::default());
    let started = live.emit(SignalKind::PlayStarted, start());
    live.settings.set_tier(Tier::T0, false);
    assert_eq!(live.emit_rate(started, 68), EmitOutcome::Disabled);
    assert_eq!(live.bus.derived_queued_total(), 0);
}

#[test]
fn a_value_queued_before_the_category_was_switched_off_is_not_stored() {
    // Nothing flushes on its own for ten minutes: the play start and the value wait in the bus or
    // in the writer's stage while the listener switches Speed off.
    let live = Live::new(WriterConfig {
        flush_interval: Duration::from_secs(600),
        ..WriterConfig::default()
    });
    let started = live.emit(SignalKind::PlayStarted, start());
    assert_eq!(live.emit_rate(started, 68), EmitOutcome::Queued);
    live.settings.set_category(SignalCategory::Speed, false);
    // The outcome ends the play and makes the writer commit at once.
    live.emit(SignalKind::Completed, completed(1_800));
    live.wait_for("the play is written", |s| s.plays_written == 1);
    let stats = live.writer.stats();
    assert_eq!(stats.derived_stored, 0, "{stats:?}");
    assert_eq!(
        live.bus.derived_withdrawn() + stats.derived_withdrawn,
        1,
        "dropped by the drain or by the stage, one way or the other"
    );
    assert_eq!(rates(&live.db.reader()), [(started.as_i64() - T0, None)]);
}

// ---------------------------------------------------------------- wipes

/// Plays of a recording each, with a rate on every one: the fixture of the wipe tests.
fn seeded(plays: usize) -> Rig {
    let mut rig = Rig::new();
    for n in 0..plays {
        let rec = rig.ids.recording();
        let at = 1_000 + i64::try_from(n).unwrap() * 400_000;
        let events = rig.play(at, rec);
        let value = rig.rate_of(rig.session, rec, at, 50 + u8::try_from(n).unwrap());
        rig.write(&events, &[value]);
        rig.made.push((rec, at));
    }
    rig
}

fn wiper(rig: &Rig) -> SignalWiper {
    SignalWiper::open(&rig.db.path).unwrap()
}

fn rate_column(rig: &Rig) -> Vec<Option<i64>> {
    rig.rates().into_iter().map(|(_, rate)| rate).collect()
}

#[test]
fn a_speed_wipe_clears_the_rate_and_keeps_the_play() {
    let mut rig = seeded(3);
    assert_eq!(rate_column(&rig), [Some(50), Some(51), Some(52)]);
    let report = wiper(&rig)
        .wipe_signal_categories(&[SignalCategory::Speed])
        .unwrap();
    assert_eq!(report.plays_cleared, 3, "the rate was all that changed");
    assert_eq!(report.plays_deleted, 0);
    assert_eq!(rate_column(&rig), [None, None, None]);
    let conn = rig.db.reader();
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM play WHERE end_reason = 0 AND listened_ds = 1800 AND end_frac = 255"
        ),
        3,
        "how the plays ended is untouched"
    );
    // A value that arrives after the wipe is stored again: the column is cleared, not retired.
    let (rec, at) = rig.made[0];
    let value = rig.rate_of(rig.session, rec, at, 77);
    rig.write(&[], &[value]);
    assert_eq!(rate_column(&rig), [Some(77), None, None]);
}

/// T0 holds PlayStarted (which makes the play row) and RateChanged (the rate's input), so the
/// plays go, and no rate survives. (The rule that the input's tier alone reaches the column is
/// pinned in signal_wipe.rs, where the kinds of a wipe can be chosen freely.)
#[test]
fn a_t0_wipe_leaves_no_play_and_so_no_rate() {
    for through in [0, 1] {
        let rig = seeded(2);
        let mut w = wiper(&rig);
        if through == 0 {
            w.wipe_signal_tiers(&[Tier::T0]).unwrap();
        } else {
            w.wipe_tiers(&[Tier::T0]).unwrap();
        }
        assert_eq!(count(&rig.db.reader(), "SELECT count(*) FROM play"), 0);
    }
}

#[test]
fn a_wipe_of_what_the_rate_is_not_derived_from_keeps_it() {
    for (what, run) in [
        (
            "T1",
            (|w: &mut SignalWiper| w.wipe_signal_tiers(&[Tier::T1]).map(|_| ()))
                as fn(&mut SignalWiper) -> Result<(), tinwave_db::DbError>,
        ),
        ("T2", |w| w.wipe_signal_tiers(&[Tier::T2]).map(|_| ())),
        ("T3", |w| w.wipe_signal_tiers(&[Tier::T3]).map(|_| ())),
        ("Volume", |w| {
            w.wipe_signal_categories(&[SignalCategory::Volume])
                .map(|_| ())
        }),
        ("Browsing", |w| {
            w.wipe_signal_categories(&[SignalCategory::Browsing])
                .map(|_| ())
        }),
        ("OutputSound", |w| {
            w.wipe_signal_categories(&[SignalCategory::OutputSound])
                .map(|_| ())
        }),
        ("Session", |w| {
            w.wipe_signal_categories(&[SignalCategory::Session])
                .map(|_| ())
        }),
    ] {
        let rig = seeded(2);
        run(&mut wiper(&rig)).unwrap();
        assert_eq!(rate_column(&rig), [Some(50), Some(51)], "{what}");
    }
}

#[test]
fn the_other_categories_that_feed_the_play_keep_the_rate_but_not_their_own_columns() {
    // Seek/skip/replay clears how the plays ended; the rate is not derived from it.
    let rig = seeded(2);
    wiper(&rig)
        .wipe_signal_categories(&[SignalCategory::SeekSkipReplay])
        .unwrap();
    assert_eq!(rate_column(&rig), [Some(50), Some(51)]);
    let unknown = EndReason::Unknown.code();
    assert_eq!(
        count(
            &rig.db.reader(),
            &format!("SELECT count(*) FROM play WHERE end_reason = {unknown}")
        ),
        2
    );
}

#[test]
fn the_queue_and_library_wipe_deletes_the_plays_with_their_rate() {
    let rig = seeded(2);
    let report = wiper(&rig)
        .wipe_signal_categories(&[SignalCategory::QueueLibrary])
        .unwrap();
    assert_eq!(report.plays_deleted, 2);
    assert_eq!(count(&rig.db.reader(), "SELECT count(*) FROM play"), 0);
}

#[test]
fn wipe_everything_removes_the_plays_and_so_their_rate() {
    let rig = seeded(2);
    wiper(&rig).wipe_everything().unwrap();
    assert_eq!(count(&rig.db.reader(), "SELECT count(*) FROM play"), 0);
}

#[test]
fn a_wipe_that_finds_no_rate_to_clear_reports_no_play_cleared() {
    let mut rig = Rig::new();
    let rec = rig.ids.recording();
    let events = rig.play(1_000, rec);
    rig.write(&events, &[]);
    let report = wiper(&rig)
        .wipe_signal_categories(&[SignalCategory::Speed])
        .unwrap();
    assert_eq!(
        report.plays_cleared, 0,
        "NULL stays NULL: no row is written"
    );
}
