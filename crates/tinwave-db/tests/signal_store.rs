//! `SignalStore` against a real SQLite file in a temp dir: schema, migrations, the batch
//! transaction, dedupe, counters, dictionaries and the typed sink error.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::error::Error;
use std::time::Duration;

use common::{count, favorited, Ev, Ids, TempDb, T0};
use tinwave_core::{TzOffsetMin, UtcMillis};
use tinwave_db::{DbError, SignalStore, StoreConfig, SIGNALS_MIGRATIONS};
use tinwave_signals::{
    BusConfig, CounterEntity, CounterRow, EntityKind, Fraction, LocalDay, SignalBus, SignalEvent,
    SignalKind, SignalPayload, SignalSettings, SignalsError,
};

fn day() -> LocalDay {
    LocalDay::from_utc(UtcMillis::new(T0), TzOffsetMin::UTC)
}

fn counter(
    kind: SignalKind,
    entity_kind: EntityKind,
    uuid: [u8; 16],
    n: u64,
    ms: u64,
    scrolled: u8,
) -> CounterRow {
    CounterRow {
        day: day(),
        kind,
        entity_kind,
        entity: uuid,
        count: n,
        total_ms: ms,
        max_scrolled: Fraction::from_raw(scrolled),
    }
}

// ---------------------------------------------------------------- migrations and schema

#[test]
fn migrations_apply_on_an_empty_database_and_are_idempotent() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let r = db.reader();
    let tables = |r: &rusqlite::Connection| -> Vec<String> {
        let mut stmt = r
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let first = tables(&r);
    for expected in [
        "context_snapshot",
        "dict_device",
        "dict_entity",
        "dict_recording",
        "dict_source",
        "play",
        "rollup_monthly",
        "schema_migration",
        "session",
        "signal_event",
        "ui_counter",
    ] {
        assert!(
            first.iter().any(|t| t == expected),
            "missing table {expected}"
        );
    }
    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 1);

    // Running the runner again, and reopening the file, changes nothing.
    let report = store.migrate().unwrap();
    assert_eq!((report.applied, report.version), (0, 1));
    drop(store);
    let _again = SignalStore::open(&db.path).unwrap();
    assert_eq!(tables(&r), first);
    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 1);
    assert_eq!(SIGNALS_MIGRATIONS.len(), 1);
}

#[test]
fn schema_matches_database_md() {
    let db = TempDb::new();
    let _store = SignalStore::open(&db.path).unwrap();
    let r = db.reader();
    let columns = |table: &str| -> Vec<(String, i64)> {
        let mut stmt = r.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        stmt.query_map([], |row| Ok((row.get(1)?, row.get(5)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let names = |table: &str| -> Vec<String> { columns(table).into_iter().map(|c| c.0).collect() };
    assert_eq!(
        names("signal_event"),
        [
            "device",
            "seq",
            "session",
            "ts_off",
            "kind",
            "recording",
            "source",
            "pos_ds",
            "payload",
            "ctx"
        ]
    );
    assert_eq!(names("dict_entity"), ["id", "kind", "uuid"]);
    assert_eq!(
        names("session"),
        ["id", "uuid", "device", "started_ts", "tz_offset_min"]
    );
    // signal_event is WITHOUT ROWID, keyed (device, seq): the sync dedupe key is the primary key.
    let event_pk: Vec<_> = columns("signal_event")
        .into_iter()
        .filter(|c| c.1 > 0)
        .collect();
    assert_eq!(event_pk, [("device".to_owned(), 1), ("seq".to_owned(), 2)]);
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM pragma_index_list('signal_event') WHERE origin = 'u'"
        ),
        0,
        "no separate UNIQUE index"
    );
    let index_columns: Vec<String> = {
        let mut stmt = r
            .prepare("SELECT name FROM pragma_index_info('signal_event_rec_ts') ORDER BY seqno")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(index_columns, ["recording", "session", "ts_off"]);
    assert!(
        r.prepare("SELECT rowid FROM signal_event").is_err(),
        "WITHOUT ROWID"
    );
    // ui_counter: (day, kind, entity_kind, entity) is the primary key, in that order.
    assert_eq!(
        names("ui_counter"),
        [
            "day",
            "kind",
            "entity_kind",
            "entity",
            "count",
            "total_ms",
            "max_scrolled"
        ]
    );
    let pk: Vec<_> = columns("ui_counter")
        .into_iter()
        .filter(|c| c.1 > 0)
        .collect();
    assert_eq!(
        pk,
        [
            ("day".to_owned(), 1),
            ("kind".to_owned(), 2),
            ("entity_kind".to_owned(), 3),
            ("entity".to_owned(), 4)
        ]
    );
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'signal_event_rec_ts'"
        ),
        1
    );
}

#[test]
fn the_database_is_wal_with_incremental_auto_vacuum() {
    let db = TempDb::new();
    let _store = SignalStore::open(&db.path).unwrap();
    let r = db.reader();
    let mode: String = r
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal", "WAL is persistent in the file");
    assert_eq!(count(&r, "PRAGMA auto_vacuum"), 2, "incremental");
    assert_eq!(count(&r, "PRAGMA page_size"), 4096);
}

// ---------------------------------------------------------------- events

#[test]
fn an_event_round_trips_through_the_surrogate_keys() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(1);
    let (device, session, rec, src) = (ids.device(), ids.session(), ids.recording(), ids.source());
    let event = Ev {
        device,
        seq: 7,
        ts: T0 + 5,
        session,
        recording: Some(rec),
        source: Some(src),
        pos_ds: Some(1_234),
        payload: SignalPayload::Favorited,
    }
    .build();
    let mut with_payload = event;
    with_payload.id.seq = 8;
    with_payload.payload = SignalPayload::Paused(tinwave_signals::PausedPayload {
        input: tinwave_signals::InputSource::Keyboard,
    });
    with_payload.ctx = Some(tinwave_signals::ContextSnapshotId(3));

    let report = store.write_batch(&[event, with_payload], &[]).unwrap();
    assert_eq!((report.events_inserted, report.events_duplicate), (2, 0));

    let r = db.reader();
    let row = |seq: i64| {
        r.query_row(
            "SELECT d.uuid, ss.started_ts + e.ts_off, e.kind, rec.uuid, s.uuid, e.pos_ds, e.payload, e.ctx, e.session
             FROM signal_event e
             JOIN dict_device d ON d.id = e.device
             JOIN session ss ON ss.id = e.session
             LEFT JOIN dict_recording rec ON rec.id = e.recording
             LEFT JOIN dict_source s ON s.id = e.source
             WHERE e.seq = ?1",
            [seq],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, u8>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Option<u32>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .unwrap()
    };
    let a = row(7);
    assert_eq!(a.0, device.as_bytes());
    assert_eq!(a.1, T0 + 5);
    assert_eq!(a.2, SignalKind::Favorited.discriminant());
    assert_eq!(a.3, rec.as_bytes());
    assert_eq!(a.4, src.as_bytes());
    assert_eq!(a.5, Some(1_234));
    assert_eq!(a.6, None, "a kind without fields stores NULL");
    assert_eq!(a.7, None);
    let b = row(8);
    assert_eq!(
        b.6,
        with_payload
            .encode_payload()
            .unwrap()
            .map(|p| p.as_bytes().to_vec())
    );
    assert_eq!(b.7, Some(3));
    assert_eq!(a.8, b.8, "same session row");

    // The session row: start from the UUIDv7, offset from the config.
    let (started, tz): (i64, i64) = r
        .query_row("SELECT started_ts, tz_offset_min FROM session", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(started, session.created_at().as_i64());
    assert_eq!(tz, 0);
    let stored_uuid: Vec<u8> = r
        .query_row("SELECT uuid FROM session", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored_uuid, session.as_bytes());
    assert_eq!(
        count(&r, "SELECT ts_off FROM signal_event WHERE seq = 7"),
        5,
        "stored relative to the session start"
    );
    assert_eq!(count(&r, "SELECT count(*) FROM session"), 1);
    assert_eq!(count(&r, "SELECT count(*) FROM dict_device"), 1);
}

#[test]
fn a_session_found_by_uuid_after_a_restart_gets_no_second_row() {
    let db = TempDb::new();
    let mut ids = Ids::new(31);
    let (device, rec, session) = (ids.device(), ids.recording(), ids.session());
    let mut store = SignalStore::open(&db.path).unwrap();
    store
        .write_batch(&[favorited(device, session, rec, 0)], &[])
        .unwrap();
    drop(store);
    // A new run: empty LRU, same session id (for example a queued event of a long session).
    let mut store = SignalStore::open(&db.path).unwrap();
    store
        .write_batch(&[favorited(device, session, rec, 1)], &[])
        .unwrap();
    let r = db.reader();
    assert_eq!(count(&r, "SELECT count(*) FROM session"), 1);
    assert_eq!(
        count(&r, "SELECT count(DISTINCT session) FROM signal_event"),
        1
    );
    assert_eq!(
        count(&r, "SELECT e.ts_off FROM signal_event e WHERE e.seq = 1"),
        1_000,
        "relative to the stored session start"
    );
}

#[test]
fn a_new_session_row_uses_the_current_timezone_offset() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let tz = store.session_tz();
    let mut ids = Ids::new(2);
    let (device, rec) = (ids.device(), ids.recording());
    let (s1, s2) = (ids.session(), ids.session());
    tz.set(TzOffsetMin::new(120).unwrap());
    store
        .write_batch(&[favorited(device, s1, rec, 0)], &[])
        .unwrap();
    tz.set(TzOffsetMin::new(-300).unwrap());
    store
        .write_batch(&[favorited(device, s2, rec, 1)], &[])
        .unwrap();
    let r = db.reader();
    assert_eq!(
        count(&r, "SELECT tz_offset_min FROM session ORDER BY id LIMIT 1"),
        120
    );
    assert_eq!(
        count(
            &r,
            "SELECT tz_offset_min FROM session ORDER BY id DESC LIMIT 1"
        ),
        -300
    );
}

#[test]
fn a_duplicate_device_seq_is_ignored() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(3);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let batch: Vec<SignalEvent> = (0..10)
        .map(|s| favorited(device, session, rec, s))
        .collect();

    let first = store.write_batch(&batch, &[]).unwrap();
    assert_eq!((first.events_inserted, first.events_duplicate), (10, 0));
    let again = store.write_batch(&batch, &[]).unwrap();
    assert_eq!((again.events_inserted, again.events_duplicate), (0, 10));

    // Overlapping batch: only the new ones are inserted, and a later `ts` for an old key
    // does not overwrite it.
    let mut overlap: Vec<SignalEvent> = (5..15)
        .map(|s| favorited(device, session, rec, s))
        .collect();
    overlap[0].ts = UtcMillis::new(T0 + 999_999);
    let third = store.write_batch(&overlap, &[]).unwrap();
    assert_eq!((third.events_inserted, third.events_duplicate), (5, 5));
    let r = db.reader();
    assert_eq!(count(&r, "SELECT count(*) FROM signal_event"), 15);
    assert_eq!(
        count(&r, "SELECT ts_off FROM signal_event WHERE seq = 5"),
        5_000
    );

    // Same seq from another device is a different event.
    let other = ids.device();
    store
        .write_batch(&[favorited(other, session, rec, 3)], &[])
        .unwrap();
    assert_eq!(count(&r, "SELECT count(*) FROM signal_event"), 16);
}

#[test]
fn first_seq_is_one_more_than_the_highest_stored_seq_of_the_device() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(4);
    let (device, other, session, rec) =
        (ids.device(), ids.device(), ids.session(), ids.recording());
    assert_eq!(store.first_seq(device).unwrap(), 0, "empty database");
    // Gaps are normal; the maximum counts, not the number of rows.
    let batch = [3, 4, 41, 17].map(|s| favorited(device, session, rec, s));
    store.write_batch(&batch, &[]).unwrap();
    store
        .write_batch(&[favorited(other, session, rec, 100)], &[])
        .unwrap();
    assert_eq!(store.first_seq(device).unwrap(), 42);
    assert_eq!(store.first_seq(other).unwrap(), 101);
    assert_eq!(store.first_seq(ids.device()).unwrap(), 0, "unknown device");

    // A new run starts above everything stored: the bus hands out 42, 43, ...
    drop(store);
    let store = SignalStore::open(&db.path).unwrap();
    let mut cfg = BusConfig::new(device);
    cfg.first_seq = store.first_seq(device).unwrap();
    let (bus, mut drain) = SignalBus::channel(
        cfg,
        ids.clock.clone(),
        std::sync::Arc::new(SignalSettings::new()),
    )
    .unwrap();
    let _ = bus
        .emit_with(SignalKind::Favorited, |s| {
            Ev {
                device,
                seq: s.id.seq,
                ts: s.ts.as_i64(),
                session,
                recording: Some(rec),
                source: None,
                pos_ds: None,
                payload: SignalPayload::Favorited,
            }
            .build()
        })
        .unwrap();
    let mut sink = SignalStore::open(&db.path).unwrap();
    drain.pump(&mut sink).unwrap();
    assert_eq!(
        count(
            &db.reader(),
            "SELECT max(seq) FROM signal_event WHERE device = 1"
        ),
        42
    );
}

// ---------------------------------------------------------------- counters

#[test]
fn counter_rows_are_deltas_added_with_the_larger_scroll_depth_kept() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let uuid = [7; 16];
    let dwell = |n, ms, scrolled| {
        counter(
            SignalKind::PageDwell,
            EntityKind::Album,
            uuid,
            n,
            ms,
            scrolled,
        )
    };
    store.write_batch(&[], &[dwell(2, 30_000, 100)]).unwrap();
    store.write_batch(&[], &[dwell(3, 12_000, 40)]).unwrap();
    store.write_batch(&[], &[dwell(1, 500, 200)]).unwrap();
    let r = db.reader();
    let (n, ms, scrolled): (i64, i64, i64) = r
        .query_row(
            "SELECT count, total_ms, max_scrolled FROM ui_counter",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (n, ms, scrolled),
        (6, 42_500, 200),
        "scroll depth never goes down"
    );
    assert_eq!(count(&r, "SELECT count(*) FROM ui_counter"), 1);
}

#[test]
fn counters_resolve_through_the_dictionary_the_entity_kind_selects() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(5);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    // An event about the recording first, so the counter must reuse its dictionary id.
    store
        .write_batch(&[favorited(device, session, rec, 0)], &[])
        .unwrap();

    let same = *rec.as_bytes();
    let rows = [
        counter(SignalKind::Impression, EntityKind::Track, same, 5, 0, 0),
        // The same 16 bytes under another kind is another row, in dict_entity.
        counter(SignalKind::Impression, EntityKind::Album, same, 2, 0, 0),
        counter(
            SignalKind::ArtworkHover,
            EntityKind::Artist,
            [9; 16],
            1,
            900,
            0,
        ),
    ];
    store.write_batch(&[], &rows).unwrap();

    let r = db.reader();
    assert_eq!(
        count(&r, "SELECT count(*) FROM dict_recording"),
        1,
        "track reuses the event's row"
    );
    assert_eq!(count(&r, "SELECT count(*) FROM dict_entity"), 2);
    let kind_of = |uuid: &[u8]| -> i64 {
        r.query_row(
            "SELECT kind FROM dict_entity WHERE uuid = ?1",
            [uuid],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(kind_of(&same), i64::from(EntityKind::Album.code()));
    assert_eq!(kind_of(&[9; 16]), i64::from(EntityKind::Artist.code()));
    assert_eq!(count(&r, "SELECT count(*) FROM ui_counter"), 3);
    let track_count: i64 = r
        .query_row(
            "SELECT c.count FROM ui_counter c JOIN dict_recording d ON d.id = c.entity              WHERE c.entity_kind = 0 AND d.uuid = ?1",
            [&same[..]],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(track_count, 5);
    // Delta rows for the same key add up across batches.
    store
        .write_batch(
            &[],
            &[counter(
                SignalKind::Impression,
                EntityKind::Album,
                same,
                1,
                0,
                0,
            )],
        )
        .unwrap();
    assert_eq!(
        count(
            &r,
            "SELECT c.count FROM ui_counter c JOIN dict_entity d ON d.id = c.entity \
                   WHERE c.entity_kind = 1"
        ),
        3
    );
}

// ---------------------------------------------------------------- one transaction per batch

/// Makes every insert into `table` fail with a real SQLite error, from a second connection:
/// no hook in the production code.
fn poison(db: &TempDb, table: &str) {
    db.reader()
        .execute_batch(&format!(
            "CREATE TRIGGER inject_failure BEFORE INSERT ON {table} \
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;"
        ))
        .unwrap();
}

fn heal(db: &TempDb) {
    db.reader()
        .execute_batch("DROP TRIGGER inject_failure;")
        .unwrap();
}

#[test]
fn a_failure_mid_batch_rolls_back_events_counters_and_dictionary_rows() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(6);
    let (device, session) = (ids.device(), ids.session());
    let (rec_a, rec_b) = (ids.recording(), ids.recording());
    let events: Vec<SignalEvent> = (0..5)
        .map(|s| favorited(device, session, rec_a, s))
        .collect();
    let counters = [
        counter(
            SignalKind::Impression,
            EntityKind::Track,
            *rec_a.as_bytes(),
            4,
            0,
            0,
        ),
        counter(
            SignalKind::PageDwell,
            EntityKind::Album,
            [3; 16],
            1,
            2_000,
            90,
        ),
    ];

    // The failure comes after the events, the session and the dictionary rows were inserted.
    poison(&db, "ui_counter");
    let error = store.write_batch(&events, &counters).unwrap_err();
    assert!(
        error.chain().contains("injected failure"),
        "{}",
        error.chain()
    );
    let r = db.reader();
    for table in [
        "signal_event",
        "ui_counter",
        "session",
        "dict_device",
        "dict_recording",
        "dict_entity",
    ] {
        assert_eq!(
            count(&r, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table} rolled back"
        );
    }
    assert_eq!(
        store.cached_entries(),
        0,
        "nothing rolled back may stay in the LRU"
    );

    // The retry (the bus offers the same rows again) counts exactly once.
    heal(&db);
    let report = store.write_batch(&events, &counters).unwrap();
    assert_eq!((report.events_inserted, report.counter_rows), (5, 2));
    assert_eq!(count(&r, "SELECT count(*) FROM signal_event"), 5);
    assert_eq!(
        count(&r, "SELECT sum(count) FROM ui_counter"),
        5,
        "4 + 1, not doubled"
    );
    assert_eq!(count(&r, "SELECT count(*) FROM dict_recording"), 1);

    // A rolled-back dictionary id must not be remembered: rec_b would otherwise inherit rec_a's
    // rolled-back id and later events would point at the wrong recording.
    poison(&db, "signal_event");
    store
        .write_batch(&[favorited(device, session, rec_b, 50)], &[])
        .unwrap_err();
    heal(&db);
    store
        .write_batch(&[favorited(device, session, rec_b, 51)], &[])
        .unwrap();
    store
        .write_batch(&[favorited(device, session, rec_a, 52)], &[])
        .unwrap();
    let uuid_of_seq = |seq: i64| -> Vec<u8> {
        r.query_row(
            "SELECT d.uuid FROM signal_event e JOIN dict_recording d ON d.id = e.recording \
             WHERE e.seq = ?1",
            [seq],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(uuid_of_seq(0), rec_a.as_bytes());
    assert_eq!(uuid_of_seq(51), rec_b.as_bytes());
    assert_eq!(uuid_of_seq(52), rec_a.as_bytes());
}

#[test]
fn a_failed_commit_leaves_the_store_usable_and_the_bus_retry_counts_once() {
    // The same scenario through the SignalSink contract and a real bus.
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(7);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let (bus, mut drain) = SignalBus::channel(
        BusConfig::new(device),
        ids.clock.clone(),
        std::sync::Arc::new(SignalSettings::new()),
    )
    .unwrap();
    for _ in 0..3 {
        let _ = bus
            .emit_with(SignalKind::Favorited, |s| {
                favorited(device, session, rec, s.id.seq)
            })
            .unwrap();
    }
    let _ = bus
        .counters()
        .impression(CounterEntity::new(EntityKind::Playlist, [4; 16]));

    poison(&db, "ui_counter");
    let err = drain.pump(&mut store).unwrap_err();
    assert!(matches!(err, SignalsError::Sink { .. }));
    let r = db.reader();
    assert_eq!(count(&r, "SELECT count(*) FROM signal_event"), 0);

    // Typed error kept inside tinwave-db, whole chain in the string.
    let typed = store.last_error().expect("typed error kept");
    let source = typed.source().expect("source chain kept");
    assert!(source.downcast_ref::<rusqlite::Error>().is_some());
    let SignalsError::Sink { reason } = err else {
        unreachable!()
    };
    assert!(
        reason.contains("committing the batch") || reason.contains("adding a counter row"),
        "{reason}"
    );
    assert!(reason.contains("injected failure"), "{reason}");

    heal(&db);
    let report = drain.pump(&mut store).unwrap();
    assert_eq!((report.events, report.counter_rows), (3, 1));
    assert!(store.last_error().is_none());
    assert_eq!(count(&r, "SELECT count(*) FROM signal_event"), 3);
    assert_eq!(count(&r, "SELECT sum(count) FROM ui_counter"), 1);
}

#[test]
fn an_unopenable_database_is_a_typed_error() {
    let db = TempDb::new();
    let missing = db.dir.path().join("no-such-dir").join("signals.db");
    let error = SignalStore::open(&missing).unwrap_err();
    assert!(matches!(error, DbError::Open { .. }), "{error}");
    assert!(
        error.source().is_some(),
        "SQLite's error is kept as the source"
    );
}

#[test]
fn the_dictionary_cache_is_bounded() {
    let db = TempDb::new();
    let cfg = StoreConfig {
        dict_cache_capacity: 8,
        session_cache_capacity: 2,
        ..StoreConfig::default()
    };
    let mut store = SignalStore::open_with(&db.path, cfg).unwrap();
    let mut ids = Ids::new(8);
    let (device, session) = (ids.device(), ids.session());
    // 200 distinct recordings through an 8-entry cache: all resolve correctly, the cache stays small.
    let recs: Vec<_> = (0..200).map(|_| ids.recording()).collect();
    for (i, rec) in recs.iter().enumerate() {
        store
            .write_batch(
                &[favorited(device, session, *rec, u64::try_from(i).unwrap())],
                &[],
            )
            .unwrap();
        assert!(store.cached_entries() <= 10);
    }
    // Evicted entries are found again in the database, not re-inserted.
    store
        .write_batch(&[favorited(device, session, recs[0], 1_000)], &[])
        .unwrap();
    let r = db.reader();
    assert_eq!(count(&r, "SELECT count(*) FROM dict_recording"), 200);
    assert_eq!(
        count(&r, "SELECT count(DISTINCT recording) FROM signal_event"),
        200
    );
}

// ---------------------------------------------------------------- WAL checkpoints

#[test]
fn the_wal_is_checkpointed_when_idle_and_large_and_never_inside_a_commit() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut listener = common::Listener::new(21, 2_000);
    let wal = |p: &std::path::Path| {
        std::fs::metadata(format!("{}-wal", p.display())).map_or(0, |m| m.len())
    };

    // Commits alone never checkpoint: SQLite's automatic checkpoint is off, so the WAL only grows.
    let mut written = 0;
    while store.wal_bytes() < 9 * 1024 * 1024 {
        let batch = listener.events(256);
        written += batch.len();
        store.write_batch(&batch, &[]).unwrap();
        assert!(
            written < 400_000,
            "the WAL should have reached 9 MiB by now"
        );
    }
    assert_eq!(store.wal_bytes(), wal(&db.path));

    // Too soon after opening, and not idle: nothing happens.
    assert!(
        !store.checkpoint_if_due(true).unwrap(),
        "less than 2 s since open"
    );
    std::thread::sleep(Duration::from_millis(2_100));
    assert!(
        !store.checkpoint_if_due(false).unwrap(),
        "events are waiting and the WAL is below 64 MiB"
    );
    assert!(store.wal_bytes() >= 9 * 1024 * 1024);

    // Idle, large and old enough: the WAL is folded into the database and truncated.
    assert!(store.checkpoint_if_due(true).unwrap());
    assert_eq!(store.wal_bytes(), 0);
    let r = db.reader();
    assert_eq!(
        count(&r, "SELECT count(*) FROM signal_event"),
        i64::try_from(written).unwrap()
    );
    // Nothing to do now.
    assert!(!store.checkpoint_if_due(true).unwrap());
    store.write_batch(&listener.events(10), &[]).unwrap();
    assert!(store.wal_bytes() > 0, "the next commit starts a new WAL");
}
