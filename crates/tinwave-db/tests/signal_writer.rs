//! The writer thread: flush policy, retry, shutdown, wiring with the bus.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{count, favorited, paused, Ids, TempDb};
use rusqlite::Connection;
use tinwave_core::{DeviceId, RecordingId, SessionId, TzOffsetMin};
use tinwave_db::{CheckpointPolicy, SignalStore, SignalWriter, StoreConfig, WriterConfig};
use tinwave_signals::{
    BusConfig, CounterEntity, EntityKind, SignalBus, SignalDrain, SignalKind, SignalSettings,
};

struct Rig {
    db: TempDb,
    bus: SignalBus,
    writer: SignalWriter,
    device: DeviceId,
    session: SessionId,
    rec: RecordingId,
}

/// A bus + writer on a temp database. `interval` and `max_events` are the flush policy.
fn rig(interval: Duration, flush_events: usize) -> Rig {
    rig_with(interval, flush_events, StoreConfig::default())
}

/// [`rig`] with explicit store settings (for example a fast checkpoint policy).
fn rig_with(interval: Duration, flush_events: usize, store_config: StoreConfig) -> Rig {
    let db = TempDb::new();
    let store = SignalStore::open_with(&db.path, store_config).unwrap();
    let mut ids = Ids::new(11);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let mut cfg = BusConfig::new(device);
    cfg.first_seq = store.first_seq(device).unwrap();
    let (bus, drain) =
        SignalBus::channel(cfg, ids.clock.clone(), Arc::new(SignalSettings::new())).unwrap();
    let config = WriterConfig {
        flush_interval: interval,
        flush_events,
        stop_poll: Duration::from_millis(20),
    };
    let writer = SignalWriter::spawn(store, drain, config).unwrap();
    Rig {
        db,
        bus,
        writer,
        device,
        session,
        rec,
    }
}

impl Rig {
    fn favorite(&self) {
        let (d, s, r) = (self.device, self.session, self.rec);
        let _ = self
            .bus
            .emit_with(SignalKind::Favorited, |st| favorited(d, s, r, st.id.seq))
            .unwrap();
    }

    fn pause(&self) {
        let (d, s, r) = (self.device, self.session, self.rec);
        let _ = self
            .bus
            .emit_with(SignalKind::Paused, |st| paused(d, s, r, st.id.seq))
            .unwrap();
    }

    fn events(&self, conn: &Connection) -> i64 {
        count(conn, "SELECT count(*) FROM signal_event")
    }
}

/// Polls until `check` holds or `within` passes; returns whether it held.
fn eventually(within: Duration, mut check: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if check() {
            return true;
        }
        if start.elapsed() > within {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

const LONG: Duration = Duration::from_secs(600);

#[test]
fn shutdown_flushes_everything_the_bus_accepted() {
    // Neither the interval nor the size threshold would flush: only the shutdown does.
    let rig = rig(LONG, 100_000);
    for _ in 0..1_500 {
        rig.favorite();
    }
    let _ = rig
        .bus
        .counters()
        .impression(CounterEntity::new(EntityKind::Track, [1; 16]));
    let accepted = rig.bus.queued_total();
    assert_eq!(accepted, 1_500);
    assert_eq!(rig.bus.dropped_total(), 0);
    let conn = rig.db.reader();
    let Rig {
        db: _db,
        bus,
        writer,
        ..
    } = rig;
    let stats = writer.shutdown().unwrap();
    assert_eq!(stats.events_inserted, accepted);
    assert_eq!(stats.events_lost, 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 1_500);
    assert_eq!(
        count(&conn, "SELECT sum(count) FROM ui_counter"),
        1,
        "counters are flushed too"
    );
    assert_eq!(
        count(&conn, "SELECT max(seq) - min(seq) + 1 FROM signal_event"),
        1_500
    );
    drop(bus);
}

#[test]
fn events_are_flushed_after_the_interval() {
    let rig = rig(Duration::from_millis(150), 100_000);
    let conn = rig.db.reader();
    for _ in 0..3 {
        rig.favorite();
    }
    assert!(eventually(Duration::from_secs(5), || rig.events(&conn) == 3));
}

#[test]
fn events_wait_for_the_interval_instead_of_one_transaction_each() {
    let rig = rig(LONG, 100_000);
    let conn = rig.db.reader();
    for _ in 0..50 {
        rig.favorite();
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(rig.events(&conn), 0, "nothing forces a flush yet");
    assert_eq!(rig.writer.stats().batches_committed, 0);
    let _ = rig.writer.shutdown().unwrap();
    assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 50);
}

#[test]
fn a_full_batch_is_flushed_at_once() {
    let rig = rig(LONG, 256);
    let conn = rig.db.reader();
    for _ in 0..256 {
        rig.favorite();
    }
    assert!(eventually(Duration::from_secs(5), || rig.events(&conn) == 256));
    assert_eq!(rig.writer.stats().batches_committed, 1);
}

#[test]
fn a_play_end_is_flushed_at_once_with_what_precedes_it() {
    let rig = rig(LONG, 100_000);
    let conn = rig.db.reader();
    for _ in 0..5 {
        rig.favorite();
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rig.events(&conn), 0);
    rig.pause();
    assert!(eventually(Duration::from_secs(5), || rig.events(&conn) == 6));
}

#[test]
fn counters_alone_are_flushed_without_waiting_for_events() {
    let rig = rig(Duration::from_millis(100), 100_000);
    let conn = rig.db.reader();
    let entity = CounterEntity::new(EntityKind::Album, [2; 16]);
    for _ in 0..40 {
        let _ = rig.bus.counters().impression(entity);
    }
    assert!(eventually(Duration::from_secs(5), || {
        count(&conn, "SELECT coalesce(sum(count), 0) FROM ui_counter") == 40
    }));
    for _ in 0..2 {
        let _ = rig.bus.counters().impression(entity);
    }
    assert!(eventually(Duration::from_secs(5), || {
        count(&conn, "SELECT coalesce(sum(count), 0) FROM ui_counter") == 42
    }));
    assert_eq!(
        count(&conn, "SELECT count(*) FROM ui_counter"),
        1,
        "deltas add into one row"
    );
}

#[test]
fn a_failing_database_is_retried_and_nothing_is_lost_or_counted_twice() {
    let rig = rig(Duration::from_millis(60), 100_000);
    let conn = rig.db.reader();
    conn.execute_batch(
        "CREATE TRIGGER inject_failure BEFORE INSERT ON ui_counter \
         BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    for _ in 0..10 {
        rig.favorite();
    }
    let entity = CounterEntity::new(EntityKind::Track, *rig.rec.as_bytes());
    for _ in 0..7 {
        let _ = rig.bus.counters().impression(entity);
    }
    assert!(eventually(Duration::from_secs(5), || rig
        .writer
        .stats()
        .failed_commits
        >= 2));
    assert_eq!(
        rig.events(&conn),
        0,
        "the failed transaction left nothing behind"
    );
    let error = rig.writer.take_last_error().expect("typed error kept");
    assert!(
        error.chain().contains("injected failure"),
        "{}",
        error.chain()
    );

    conn.execute_batch("DROP TRIGGER inject_failure;").unwrap();
    assert!(eventually(Duration::from_secs(5), || rig.events(&conn) == 10));
    assert_eq!(
        count(&conn, "SELECT sum(count) FROM ui_counter"),
        7,
        "counted once"
    );
    assert!(
        rig.writer.take_last_error().is_none(),
        "cleared by the successful commit"
    );
}

#[test]
fn shutting_down_while_the_database_fails_reports_what_was_lost() {
    let rig = rig(LONG, 100_000);
    let conn = rig.db.reader();
    conn.execute_batch(
        "CREATE TRIGGER inject_failure BEFORE INSERT ON signal_event \
         BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    for _ in 0..4 {
        rig.favorite();
    }
    let error = rig.writer.shutdown().unwrap_err();
    assert!(
        error.chain().contains("injected failure"),
        "{}",
        error.chain()
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 0);
}

#[test]
fn the_writer_stops_by_itself_when_every_producer_is_gone() {
    let rig = rig(LONG, 100_000);
    let conn = rig.db.reader();
    for _ in 0..9 {
        rig.favorite();
    }
    let Rig {
        db: _db,
        bus,
        writer,
        ..
    } = rig;
    drop(bus);
    assert!(eventually(Duration::from_secs(5), || {
        count(&conn, "SELECT count(*) FROM signal_event") == 9
    }));
    let stats = writer.shutdown().unwrap();
    assert_eq!(stats.events_inserted, 9);
}

#[test]
fn a_new_run_continues_the_sequence_and_the_session_offset_follows_the_setter() {
    let db = TempDb::new();
    let mut ids = Ids::new(12);
    let (device, rec) = (ids.device(), ids.recording());
    let (s1, s2) = (ids.session(), ids.session());
    let run = |first_session: SessionId, tz: i16, n: usize| -> u64 {
        let store = SignalStore::open(&db.path).unwrap();
        let mut cfg = BusConfig::new(device);
        cfg.first_seq = store.first_seq(device).unwrap();
        let first = cfg.first_seq;
        let (bus, drain): (SignalBus, SignalDrain) =
            SignalBus::channel(cfg, ids_clock(), Arc::new(SignalSettings::new())).unwrap();
        let writer = SignalWriter::spawn(store, drain, WriterConfig::default()).unwrap();
        let tz = TzOffsetMin::new(tz).unwrap();
        bus.set_tz_offset(tz);
        writer.set_tz_offset(tz);
        for _ in 0..n {
            let _ = bus
                .emit_with(SignalKind::Favorited, |st| {
                    favorited(device, first_session, rec, st.id.seq)
                })
                .unwrap();
        }
        writer.shutdown().unwrap();
        first
    };
    assert_eq!(run(s1, 60, 5), 0);
    assert_eq!(
        run(s2, -120, 5),
        5,
        "the second run starts above everything stored"
    );
    let conn = db.reader();
    assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 10);
    assert_eq!(
        count(&conn, "SELECT count(DISTINCT seq) FROM signal_event"),
        10,
        "no seq reused"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT tz_offset_min FROM session ORDER BY id LIMIT 1"
        ),
        60
    );
    assert_eq!(
        count(
            &conn,
            "SELECT tz_offset_min FROM session ORDER BY id DESC LIMIT 1"
        ),
        -120
    );
}

fn ids_clock() -> tinwave_core::ManualClock {
    Ids::new(0).clock
}

fn wal_len(rig: &Rig) -> u64 {
    std::fs::metadata(format!("{}-wal", rig.db.path.display())).map_or(0, |m| m.len())
}

#[test]
fn an_idle_writer_checkpoints_a_non_empty_wal_within_the_interval() {
    // A play end is flushed at once, but a commit is not fsynced: with nothing more to write the
    // writer must still fold the WAL into the database on its own, or a power loss could take
    // committed play ends.
    let interval = Duration::from_millis(300);
    let store_config = StoreConfig {
        checkpoint: CheckpointPolicy {
            interval,
            min_gap: Duration::from_millis(50),
            ..CheckpointPolicy::default()
        },
        ..StoreConfig::default()
    };
    let rig = rig_with(LONG, 100_000, store_config);
    let conn = rig.db.reader();
    rig.pause();
    assert!(eventually(Duration::from_secs(5), || rig.events(&conn) == 1));
    // Nothing else is emitted: the writer is idle from here on.
    assert!(
        eventually(interval * 10 + Duration::from_secs(2), || {
            rig.writer.stats().checkpoints >= 1 && wal_len(&rig) == 0
        }),
        "the idle writer never checkpointed (stats: {:?}, WAL {} bytes)",
        rig.writer.stats(),
        wal_len(&rig)
    );
    assert_eq!(rig.events(&conn), 1, "the checkpoint kept the data");
}

#[test]
fn shutdown_drains_more_than_one_batch_that_was_still_queued() {
    // The default flush size (256). The database fails, so the writer stops pulling from the bus
    // and a thousand events pile up in its queues; then it recovers and shuts down at once: the
    // final drain must take all of them, not one batch of 256.
    let rig = rig(Duration::from_secs(5), 256);
    let conn = rig.db.reader();
    conn.execute_batch(
        "CREATE TRIGGER inject_failure BEFORE INSERT ON signal_event          BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    rig.pause();
    assert!(eventually(Duration::from_secs(5), || {
        rig.writer.stats().failed_commits >= 1
    }));
    for _ in 0..1_000 {
        rig.favorite();
    }
    assert_eq!(rig.bus.queued_total(), 1_001);
    assert_eq!(rig.bus.dropped_total(), 0);
    conn.execute_batch("DROP TRIGGER inject_failure;").unwrap();

    let Rig {
        db: _db,
        bus,
        writer,
        ..
    } = rig;
    let stats = writer.shutdown().unwrap();
    assert_eq!(stats.events_inserted, 1_001);
    assert_eq!(stats.events_lost, 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 1_001);
    drop(bus);
}
