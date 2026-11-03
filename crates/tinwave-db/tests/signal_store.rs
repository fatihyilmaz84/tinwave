//! `SignalStore` against a real SQLite file in a temp dir: schema, migrations, the batch
//! transaction, dedupe, counters, dictionaries and the typed sink error.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::error::Error;
use std::time::Duration;

use common::{count, favorited, paused, shown, store_statements_of, Ev, Ids, TempDb, T0};
use tinwave_core::{DeviceId, RecordingId, SessionId, SourceId, TzOffsetMin, UtcMillis};
use tinwave_db::{
    BatchReport, CheckpointPolicy, DbError, SignalStore, StoreConfig, SIGNALS_MIGRATIONS,
};
use tinwave_signals::{
    BusConfig, CompletedPayload, CounterEntity, CounterRow, EntityKind, Fraction, InputSource,
    LocalDay, PlayOrigin, PlayStartedPayload, PositionDs, SignalBus, SignalEvent, SignalKind,
    SignalPayload, SignalSettings, SignalSink, SignalsError,
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
        "signal_settings",
        "ui_counter",
    ] {
        assert!(
            first.iter().any(|t| t == expected),
            "missing table {expected}"
        );
    }
    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 2);

    // Running the runner again, and reopening the file, changes nothing.
    let report = store.migrate().unwrap();
    assert_eq!((report.applied, report.version), (0, 2));
    drop(store);
    let _again = SignalStore::open(&db.path).unwrap();
    assert_eq!(tables(&r), first);
    assert_eq!(count(&r, "SELECT count(*) FROM schema_migration"), 2);
    assert_eq!(SIGNALS_MIGRATIONS.len(), 2);
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
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM pragma_index_list('signal_event') WHERE origin = 'c'"
        ),
        0,
        "no secondary index on signal_event (lead decision, T-0006 review)"
    );
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
    // The tables of the block that are not touched by the writer: columns, order and keys.
    assert_eq!(names("dict_device"), ["id", "uuid"]);
    assert_eq!(names("dict_recording"), ["id", "uuid"]);
    assert_eq!(names("dict_source"), ["id", "uuid"]);
    assert_eq!(names("context_snapshot"), ["id", "hash", "key", "fields"]);
    assert_eq!(names("signal_settings"), ["id", "mask", "updated_ts"]);
    assert_eq!(
        names("play"),
        [
            "id",
            "device",
            "session",
            "recording",
            "source",
            "started_ts",
            "listened_ds",
            "end_reason",
            "end_frac",
            "flags",
            "peak_section",
            "context_key"
        ]
    );
    assert_eq!(
        names("rollup_monthly"),
        [
            "month",
            "recording",
            "plays",
            "completes",
            "skips_early",
            "skips_late",
            "fav_delta",
            "listened_s",
            "score_sum"
        ]
    );
    let rollup_pk: Vec<_> = columns("rollup_monthly")
        .into_iter()
        .filter(|c| c.1 > 0)
        .collect();
    assert_eq!(
        rollup_pk,
        [("month".to_owned(), 1), ("recording".to_owned(), 2)]
    );
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'play_rec_ts'"
        ),
        1
    );
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'signal_event_rec_ts'"
        ),
        0
    );
}

/// The SQL of the `signals.db` block in `docs/rules/database.md`, comments and whitespace removed.
fn normalized(sql: &str) -> Vec<String> {
    let no_comments: String = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    no_comments
        .split(';')
        .map(|statement| statement.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|statement| !statement.is_empty())
        .collect()
}

#[test]
fn the_migration_is_the_sql_block_of_database_md() {
    let doc = include_str!("../../../docs/rules/database.md").replace("\r\n", "\n");
    let start = doc.find("## Signal tables").expect("the signals section");
    let open = doc[start..].find("```sql\n").expect("its sql block") + start + 7;
    let close = doc[open..].find("```").expect("end of the block") + open;
    let from_doc = normalized(&doc[open..close]);
    // The doc block is every migration in order: 0001 first, then each later number appended.
    let from_migration: Vec<String> = SIGNALS_MIGRATIONS
        .iter()
        .flat_map(|m| normalized(m.sql))
        .collect();
    assert!(
        from_doc.len() >= 12,
        "found the block: {} statements",
        from_doc.len()
    );
    assert_eq!(
        from_migration, from_doc,
        "the migrations and the SQL block of database.md drifted: a change to the block is a new migration number (0001 is frozen, migrations are append-only once merged to main)"
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

// ---------------------------------------------------------------- values that do not fit

#[test]
fn values_above_i64_max_are_rejected_one_by_one_and_do_not_wedge_the_batch() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(9);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let mut huge_seq = favorited(device, session, rec, 1);
    huge_seq.id.seq = u64::MAX;
    let mut huge_ctx = favorited(device, session, rec, 2);
    huge_ctx.ctx = Some(tinwave_signals::ContextSnapshotId(u64::MAX));
    let mut fine = favorited(device, session, rec, 3);
    fine.ctx = Some(tinwave_signals::ContextSnapshotId(i64::MAX as u64));
    let rows = [
        counter(
            SignalKind::Impression,
            EntityKind::Album,
            [1; 16],
            u64::MAX,
            0,
            0,
        ),
        counter(
            SignalKind::ArtworkHover,
            EntityKind::Album,
            [1; 16],
            1,
            u64::MAX,
            0,
        ),
        counter(SignalKind::Impression, EntityKind::Album, [2; 16], 4, 0, 0),
    ];

    let report = store
        .write_batch(
            &[favorited(device, session, rec, 0), huge_seq, huge_ctx, fine],
            &rows,
        )
        .expect("a value that cannot be stored must not fail the whole batch");
    assert_eq!(report.events_inserted, 2);
    assert_eq!(report.events_rejected, 2);
    assert_eq!(report.counter_rows, 1);
    assert_eq!(report.counter_rows_rejected, 2);

    let r = db.reader();
    let seqs: Vec<i64> = {
        let mut stmt = r
            .prepare("SELECT seq FROM signal_event ORDER BY seq")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(seqs, [0, 3]);
    assert_eq!(
        count(&r, "SELECT ctx FROM signal_event WHERE seq = 3"),
        i64::MAX
    );
    assert_eq!(count(&r, "SELECT count(*) FROM ui_counter"), 1);
    assert_eq!(count(&r, "SELECT count FROM ui_counter"), 4);
}

#[test]
fn a_rejected_value_does_not_wedge_the_writer_stage() {
    // Through the sink the bus uses: the batch is accepted, so nothing is retried forever.
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(10);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let mut huge = favorited(device, session, rec, 1);
    huge.id.seq = u64::MAX;
    store
        .write(&[huge, favorited(device, session, rec, 2)], &[])
        .unwrap();
    assert!(store.last_error().is_none());
    assert_eq!(store.last_report().events_rejected, 1);
    assert_eq!(count(&db.reader(), "SELECT count(*) FROM signal_event"), 1);
}

// ---------------------------------------------------------------- time relative to the session

#[test]
fn an_event_before_its_sessions_start_gets_a_negative_offset_that_round_trips() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(11);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    // The session's UUIDv7 is stamped T0; the event claims to be 5 s older (clock step back).
    let early = Ev {
        device,
        seq: 0,
        ts: T0 - 5_000,
        session,
        recording: Some(rec),
        source: None,
        pos_ds: None,
        payload: SignalPayload::Favorited,
    }
    .build();
    store.write_batch(&[early], &[]).unwrap();
    let r = db.reader();
    assert_eq!(count(&r, "SELECT ts_off FROM signal_event"), -5_000);
    assert_eq!(
        count(
            &r,
            "SELECT s.started_ts + e.ts_off FROM signal_event e JOIN session s ON s.id = e.session"
        ),
        T0 - 5_000,
        "absolute time is started_ts + ts_off"
    );
}

// ---------------------------------------------------------------- WAL checkpoints

fn policy(soft: u64, hard: u64, interval: Duration, min_gap: Duration) -> StoreConfig {
    StoreConfig {
        checkpoint: CheckpointPolicy {
            soft_limit_bytes: soft,
            hard_limit_bytes: hard,
            interval,
            min_gap,
        },
        ..StoreConfig::default()
    }
}

const NEVER: Duration = Duration::from_secs(3_600);
const HUGE: u64 = u64::MAX;

#[test]
fn commits_never_checkpoint_and_an_idle_store_checkpoints_a_large_wal() {
    let db = TempDb::new();
    let cfg = policy(64 * 1024, HUGE, NEVER, Duration::ZERO);
    let mut store = SignalStore::open_with(&db.path, cfg).unwrap();
    let mut listener = common::Listener::new(21, 2_000);
    let mut written = 0;
    let mut previous = 0;
    // SQLite's automatic checkpoint is off: the WAL only grows with commits.
    while store.wal_bytes() < 128 * 1024 {
        let batch = listener.events(256);
        written += batch.len();
        store.write_batch(&batch, &[]).unwrap();
        assert!(
            store.wal_bytes() > previous,
            "a commit never shrinks the WAL"
        );
        previous = store.wal_bytes();
    }
    assert!(
        !store.checkpoint_if_due(false).unwrap(),
        "events are waiting and the WAL is below the hard limit"
    );
    assert!(store.checkpoint_if_due(true).unwrap());
    assert_eq!(
        store.wal_bytes(),
        0,
        "folded into the database and truncated"
    );
    let r = db.reader();
    assert_eq!(
        count(&r, "SELECT count(*) FROM signal_event"),
        i64::try_from(written).unwrap()
    );
    assert!(
        !store.checkpoint_if_due(true).unwrap(),
        "nothing left to do"
    );
    store.write_batch(&listener.events(10), &[]).unwrap();
    assert!(store.wal_bytes() > 0, "the next commit starts a new WAL");
}

#[test]
fn a_small_wal_is_checkpointed_once_the_interval_has_passed() {
    let db = TempDb::new();
    let interval = Duration::from_millis(150);
    let cfg = policy(HUGE, HUGE, interval, Duration::ZERO);
    let mut store = SignalStore::open_with(&db.path, cfg).unwrap();
    let mut ids = Ids::new(12);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    store
        .write_batch(&[paused(device, session, rec, 0)], &[])
        .unwrap();
    assert!(store.wal_bytes() > 0);
    assert!(
        !store.checkpoint_if_due(false).unwrap(),
        "only an idle store checkpoints on age"
    );
    std::thread::sleep(interval + Duration::from_millis(50));
    assert!(store.checkpoint_if_due(true).unwrap());
    assert_eq!(store.wal_bytes(), 0);
}

#[test]
fn the_hard_limit_also_waits_for_the_minimum_gap() {
    let db = TempDb::new();
    let mut ids = Ids::new(13);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());

    // Over the hard limit but less than `min_gap` since the last attempt: not yet.
    let cfg = policy(HUGE, 1, NEVER, Duration::from_secs(3_600));
    let mut store = SignalStore::open_with(&db.path, cfg).unwrap();
    store
        .write_batch(&[favorited(device, session, rec, 0)], &[])
        .unwrap();
    assert!(!store.checkpoint_if_due(false).unwrap());
    assert!(!store.checkpoint_if_due(true).unwrap());
    assert!(store.wal_bytes() > 0);
    drop(store);

    // With no gap, the same WAL is checkpointed even though events are waiting.
    let cfg = policy(HUGE, 1, NEVER, Duration::ZERO);
    let mut store = SignalStore::open_with(&db.path, cfg).unwrap();
    store
        .write_batch(&[favorited(device, session, rec, 1)], &[])
        .unwrap();
    assert!(store.checkpoint_if_due(false).unwrap());
    assert_eq!(store.wal_bytes(), 0);
}

// ---------------------------------------------------------------- cost

/// The device, session and source of the cost test's events.
struct Cast {
    device: DeviceId,
    session: SessionId,
    source: SourceId,
}

/// `PlayStarted` then `Completed` for each of `recordings`, from `first_seq` on.
fn plays_of(cast: &Cast, recordings: &[RecordingId], first_seq: u64) -> Vec<SignalEvent> {
    let mut seq = first_seq;
    let mut out = Vec::new();
    for &recording in recordings {
        for payload in [
            SignalPayload::PlayStarted(PlayStartedPayload {
                origin: PlayOrigin::Queue,
                input: InputSource::Auto,
            }),
            SignalPayload::Completed(CompletedPayload {
                listened_ds: PositionDs::new(1_800).unwrap(),
                paused_ds: PositionDs::ZERO,
            }),
        ] {
            out.push(
                Ev {
                    device: cast.device,
                    seq,
                    ts: T0 + i64::try_from(seq).unwrap() * 1_000,
                    session: cast.session,
                    recording: Some(recording),
                    source: Some(cast.source),
                    pos_ds: None,
                    payload,
                }
                .build(),
            );
            seq += 1;
        }
    }
    out
}

/// A closed `signals.db` that already holds `plays` finished plays of the cast over `pool`
/// recordings: the device, the session, the source and the pool are in the dictionaries.
fn with_stored_plays(cast: &Cast, pool: &[RecordingId], plays: usize) -> TempDb {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let recordings: Vec<RecordingId> = (0..plays).map(|i| pool[i % pool.len()]).collect();
    let report = store
        .write_batch(&plays_of(cast, &recordings, 0), &[])
        .unwrap();
    assert_eq!(report.plays_written, plays);
    drop(store);
    db
}

/// The cost of a batch is a number of statements that depends on the batch, **not** on the rows
/// already stored (`SignalStore::write_batch`): the writer holds the write lock for the whole
/// transaction, so a statement per stored row (or per stored play, per dictionary row) would
/// hold it for longer the longer the app has been used. A batch is written to a new store on
/// a database that holds 3 plays and on one that holds 3 000, with a cold cache, and must run the
/// same statements, exactly the number counted here.
///
/// A batch of `n` plays, each a `PlayStarted` and a `Completed` of a recording the database has
/// not seen, of a device, session and source it has:
/// - `BEGIN IMMEDIATE`, the read of the wipe epoch and `COMMIT`: 3;
/// - the lookup of the device, the session and the source: 3;
/// - for each recording a lookup and an insert: 2 per play;
/// - 2 event inserts per play;
/// - 4 for the play row of each play: the start, the check for an existing row, the events between
///   the two and the insert (there is no context snapshot, so no lookup of its key).
///
/// So `6 + 8 n`: linear in the batch, as it must be, and in nothing else.
#[test]
fn a_batch_runs_a_statement_count_set_by_the_batch_whatever_is_already_stored() {
    let mut ids = Ids::new(74);
    let cast = Cast {
        device: ids.device(),
        session: ids.session(),
        source: ids.source(),
    };
    let pool: Vec<RecordingId> = (0..20).map(|_| ids.recording()).collect();
    let new_recordings: Vec<RecordingId> = (0..100).map(|_| ids.recording()).collect();
    let small_template = with_stored_plays(&cast, &pool, 3);
    let large_template = with_stored_plays(&cast, &pool, 3_000);
    let first_seq = 1_000_000;

    // (name, batch, the statements it must run)
    let mut cases: Vec<(String, Vec<SignalEvent>, Vec<CounterRow>, usize)> = Vec::new();
    for n in [1, 2, 10, 100] {
        cases.push((
            format!("{n} plays"),
            plays_of(&cast, &new_recordings[..n], first_seq),
            Vec::new(),
            6 + 8 * n,
        ));
    }
    // Counters only: a lookup of the entity (new), its insert and the upsert, for each row.
    for n in [1, 10] {
        let rows: Vec<CounterRow> = (0..n)
            .map(|i| {
                counter(
                    SignalKind::Impression,
                    EntityKind::Artist,
                    [u8::try_from(i).unwrap() + 1; 16],
                    1,
                    0,
                    0,
                )
            })
            .collect();
        cases.push((format!("{n} counter rows"), Vec::new(), rows, 3 + 3 * n));
    }

    let mut wrong = Vec::new();
    for (name, events, counters, expected) in &cases {
        let run = |template: &TempDb| {
            let db = template.copy();
            let mut store = SignalStore::open(&db.path).unwrap();
            let (report, sql) =
                store_statements_of(&mut store, |s| s.write_batch(events, counters));
            (report.unwrap(), sql)
        };
        let (small_report, small_sql) = run(&small_template);
        let (large_report, large_sql) = run(&large_template);

        assert_eq!(small_report, large_report, "{name}: the same batch");
        if large_sql.len() != small_sql.len() {
            wrong.push(format!(
                "{name}: the statements depend on the stored rows: {} on the small database, {} on the large one",
                small_sql.len(),
                large_sql.len()
            ));
        }
        if large_sql.len() != *expected {
            wrong.push(format!(
                "{name}: {} statements, expected {expected}: {}",
                large_sql.len(),
                shown(&large_sql)
            ));
        }
        // The transaction opens and closes inside the batch, so a lock it keeps shows.
        if large_sql.first().map(String::as_str) != Some("BEGIN IMMEDIATE")
            || large_sql.last().map(String::as_str) != Some("COMMIT")
        {
            wrong.push(format!(
                "{name}: not one transaction: {}",
                shown(&large_sql)
            ));
        }
    }

    // The same batch again (a retry, a sync): the events are all duplicates, nothing but the
    // lookups and the ignored inserts, and still not a statement per stored row.
    for template in [&small_template, &large_template] {
        let batch = plays_of(&cast, &new_recordings[..10], first_seq);
        let db = template.copy();
        SignalStore::open(&db.path)
            .unwrap()
            .write_batch(&batch, &[])
            .unwrap();
        let mut store = SignalStore::open(&db.path).unwrap();
        let (report, sql) = store_statements_of(&mut store, |s| s.write_batch(&batch, &[]));
        let report = report.unwrap();
        assert_eq!(report.events_duplicate, 20);
        assert_eq!(report.events_inserted, 0);
        // 3 + the lookups of the device, session, source and 10 recordings + 20 ignored inserts.
        if sql.len() != 3 + 13 + 20 {
            wrong.push(format!(
                "a replayed batch: {} statements: {}",
                sql.len(),
                shown(&sql)
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// An empty batch opens no transaction at all.
#[test]
fn an_empty_batch_runs_no_statement() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let (report, sql) = store_statements_of(&mut store, |s| s.write_batch(&[], &[]));
    assert_eq!(report.unwrap(), BatchReport::default());
    assert!(sql.is_empty(), "{}", shown(&sql));
}
