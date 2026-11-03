//! Wiping a signal category and wiping everything (T-0061). Every test drives the real
//! `SignalWiper` on a SQLite file; failures are injected from a second connection with a trigger
//! (as in `context_wipe.rs`), and the contention tests run the real writer path while the wipe
//! holds the write lock.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{burn_in_trigger, count, during_a_wipe, favorited, Ids, TempDb, T0};
use rusqlite::types::Value;
use rusqlite::{params, Connection};
use tinwave_db::{
    PersistentSettings, SignalStore, SignalWipeReport, SignalWiper, SignalWriter, WriterConfig,
};
use tinwave_signals::{BusConfig, SignalBus, SignalCategory, SignalKind, SignalSettings};

/// The tables a wipe may touch, with every column dumped, in a stable order.
const TABLES: [&str; 12] = [
    "signal_event",
    "ui_counter",
    "play",
    "rollup_monthly",
    "context_snapshot",
    "session",
    "dict_device",
    "dict_recording",
    "dict_source",
    "dict_entity",
    "signal_settings",
    "schema_migration",
];

type Dump = Vec<(&'static str, Vec<Vec<Value>>)>;

fn dump(conn: &Connection) -> Dump {
    TABLES
        .iter()
        .map(|table| {
            let columns = conn
                .prepare(&format!("SELECT * FROM {table}"))
                .unwrap()
                .column_count();
            let order = (1..=columns)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
                .unwrap();
            let rows = stmt
                .query_map([], |row| (0..columns).map(|i| row.get(i)).collect())
                .unwrap()
                .map(Result::unwrap)
                .collect();
            (*table, rows)
        })
        .collect()
}

fn table<'a>(dump: &'a Dump, name: &str) -> &'a Vec<Vec<Value>> {
    &dump.iter().find(|(t, _)| *t == name).unwrap().1
}

/// The kind codes of the `signal_event` rows that exist.
fn event_kinds(conn: &Connection) -> Vec<u8> {
    ints(conn, "SELECT kind FROM signal_event ORDER BY kind")
}

fn ints<T: TryFrom<i64>>(conn: &Connection, sql: &str) -> Vec<T>
where
    T::Error: std::fmt::Debug,
{
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(|v| T::try_from(v.unwrap()).unwrap())
        .collect()
}

/// A kind no build has, as a newer build might store.
const UNKNOWN_KIND: u8 = 99;

/// The number of raw-event kinds of each category in the catalog (the counters of Browsing are
/// `ui_counter` rows, not events). Written out here, not derived from `SignalCategory::of`.
fn event_kinds_of(category: SignalCategory) -> usize {
    match category {
        SignalCategory::Volume => 3,
        SignalCategory::Speed => 1,
        SignalCategory::SeekSkipReplay => 9,
        SignalCategory::QueueLibrary => 15,
        SignalCategory::Browsing => 2,
        SignalCategory::OutputSound => 4,
        SignalCategory::Session => 4,
    }
}

const COUNTER_KINDS: [SignalKind; 3] = [
    SignalKind::PageDwell,
    SignalKind::ArtworkHover,
    SignalKind::Impression,
];

/// One row of every kind in every table a wipe reaches, written with plain SQL so the fixture
/// does not share code with the writer.
fn populate(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO dict_device (id, uuid) VALUES (1, x'01');
         INSERT INTO dict_recording (id, uuid) VALUES (1, x'0a'), (2, x'0b'), (3, x'0c');
         INSERT INTO dict_source (id, uuid) VALUES (1, x'0d');
         INSERT INTO dict_entity (id, kind, uuid) VALUES (1, 1, x'0e');
         INSERT INTO session (id, uuid, device, started_ts, tz_offset_min) VALUES (1, x'0f', 1, 1000, 60);
         INSERT INTO context_snapshot (id, hash, key, fields) VALUES (1, 11, 268435457, x'00'), (2, 12, 0, x'00');",
    )
    .unwrap();
    let mut insert = conn
        .prepare(
            "INSERT INTO signal_event (device, seq, session, ts_off, kind, recording, ctx) \
             VALUES (1, ?1, 1, 0, ?2, 1, 1)",
        )
        .unwrap();
    for kind in SignalKind::events() {
        insert
            .execute(params![i64::from(kind.discriminant()), kind.discriminant()])
            .unwrap();
    }
    insert.execute(params![1_000, UNKNOWN_KIND]).unwrap();
    let mut counter = conn
        .prepare(
            "INSERT INTO ui_counter (day, kind, entity_kind, entity, count, total_ms, max_scrolled) \
             VALUES (20261101, ?1, 0, 1, 5, 100, 7)",
        )
        .unwrap();
    for kind in COUNTER_KINDS {
        counter.execute([kind.discriminant()]).unwrap();
    }
    // (id, listened_ds, end_reason, end_frac, flags, peak_section)
    conn.execute_batch(
        "INSERT INTO play (id, device, session, recording, source, started_ts, listened_ds, end_reason, \
                           end_frac, flags, peak_section, context_key) VALUES
           (1, 1, 1, 1, 1, 5000, 600, 1, 100, 2047, 3, 268435457),
           (2, 1, 1, 2, 1, 6000, 1800, 0, 255, 5, NULL, 0),
           (3, 1, 1, 3, NULL, 7000, 50, 3, 250, 0, NULL, 268435457);
         INSERT INTO rollup_monthly (month, recording, plays, completes, skips_early, skips_late, \
                                     fav_delta, listened_s, score_sum) VALUES
           (202601, 1, 10, 4, 3, 2, 2, 900, 7.5),
           (202601, 2, 0, 0, 0, 0, 3, 0, 1.0),
           (202601, 3, 5, 5, 0, 0, 0, 100, 2.0),
           (202601, 4, 0, 0, 0, 0, 0, 0, 0.0);",
    )
    .unwrap();
}

struct Fixture {
    db: TempDb,
    wiper: SignalWiper,
}

impl Fixture {
    fn new() -> Self {
        let db = TempDb::new();
        SignalStore::open(&db.path).unwrap();
        populate(&db.reader());
        let wiper = SignalWiper::open(&db.path).unwrap();
        Self { db, wiper }
    }

    fn dump(&self) -> Dump {
        dump(&self.db.reader())
    }
}

/// (listened_ds, end_reason, end_frac, flags, peak_section, recording) of a play row.
type PlayRow = (i64, i64, i64, i64, Option<i64>, i64);

/// (recording, plays, completes, skips_early, skips_late, fav_delta, listened_s, score_sum).
type RollupRow = (i64, i64, i64, i64, i64, i64, i64, f64);

fn plays(conn: &Connection) -> Vec<PlayRow> {
    let mut stmt = conn
        .prepare(
            "SELECT listened_ds, end_reason, end_frac, flags, peak_section, recording \
             FROM play ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// `(recording, plays, completes, skips_early, skips_late, fav_delta, listened_s, score_sum)`.
fn rollups(conn: &Connection) -> Vec<RollupRow> {
    let mut stmt = conn
        .prepare(
            "SELECT recording, plays, completes, skips_early, skips_late, fav_delta, listened_s, \
             score_sum FROM rollup_monthly ORDER BY recording",
        )
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

const PLAYS_BEFORE: [PlayRow; 3] = [
    (600, 1, 100, 2047, Some(3), 1),
    (1800, 0, 255, 5, None, 2),
    (50, 3, 250, 0, None, 3),
];

const ROLLUPS_BEFORE: [RollupRow; 4] = [
    (1, 10, 4, 3, 2, 2, 900, 7.5),
    (2, 0, 0, 0, 0, 3, 0, 1.0),
    (3, 5, 5, 0, 0, 0, 100, 2.0),
    (4, 0, 0, 0, 0, 0, 0, 0.0),
];

/// The kind codes of the events and counters that a wipe of `category` removes.
fn kinds_of(category: SignalCategory) -> Vec<u8> {
    SignalKind::ALL
        .iter()
        .filter(|k| SignalCategory::of(**k) == category && k.is_event())
        .map(|k| k.discriminant())
        .collect()
}

// ---------------------------------------------------------------- per category

/// Each category deletes exactly its events and counters and the derived columns the rule lists,
/// and nothing else in any table.
#[test]
fn wiping_a_category_deletes_exactly_its_events_and_the_data_derived_from_it() {
    for category in SignalCategory::ALL {
        let mut fx = Fixture::new();
        let before = fx.dump();
        let removed = kinds_of(category);
        assert_eq!(removed.len(), event_kinds_of(category), "{category:?}");

        let report = fx.wiper.wipe_signal_categories(&[category]).unwrap();

        let conn = fx.db.reader();
        // Events: every kind but this category's is still there, with the unknown kind.
        let mut expected_kinds: Vec<u8> = SignalKind::events()
            .map(|k| k.discriminant())
            .filter(|k| !removed.contains(k))
            .chain([UNKNOWN_KIND])
            .collect();
        expected_kinds.sort_unstable();
        assert_eq!(event_kinds(&conn), expected_kinds, "{category:?}: events");
        // Counters: only Browsing has any.
        let counters: Vec<u8> = ints(&conn, "SELECT kind FROM ui_counter ORDER BY kind");
        let expected_counters: Vec<u8> = COUNTER_KINDS
            .iter()
            .map(|k| k.discriminant())
            .filter(|_| category != SignalCategory::Browsing)
            .collect();
        assert_eq!(counters, expected_counters, "{category:?}: counters");

        // Play rows and rollups, per the rule (written out per category).
        let (plays_after, rollups_after, expected) = expected_derived(category);
        assert_eq!(plays(&conn), plays_after, "{category:?}: play rows");
        assert_eq!(rollups(&conn), rollups_after, "{category:?}: rollups");
        assert_eq!(report, expected(removed.len()), "{category:?}: report");

        // Nothing outside the events, counters, plays and rollups changed.
        let after = dump(&conn);
        for name in [
            "context_snapshot",
            "session",
            "dict_device",
            "dict_recording",
            "dict_source",
            "dict_entity",
            "signal_settings",
            "schema_migration",
        ] {
            assert_eq!(
                table(&after, name),
                table(&before, name),
                "{category:?}: {name}"
            );
        }
        // The play columns the rule does not name are untouched.
        let played = |d: &Dump| {
            table(d, "play")
                .iter()
                // id, device, session, recording, source, started_ts, context_key
                .map(|r| [0, 1, 2, 3, 4, 5, 11].map(|i| r[i].clone()))
                .collect::<Vec<_>>()
        };
        if category != SignalCategory::QueueLibrary {
            assert_eq!(
                played(&after),
                played(&before),
                "{category:?}: identity columns"
            );
        }
    }
}

type Derived = (Vec<PlayRow>, Vec<RollupRow>, fn(usize) -> SignalWipeReport);

/// What the derived tables hold after the wipe of `category`, and the expected report given the
/// number of event kinds removed.
fn expected_derived(category: SignalCategory) -> Derived {
    let zeroed_score = || -> Vec<_> {
        ROLLUPS_BEFORE
            .iter()
            .map(|r| (r.0, r.1, r.2, r.3, r.4, r.5, r.6, 0.0))
            .collect()
    };
    let plays_with = |flags: [i64; 3], peak: Option<i64>| -> Vec<_> {
        PLAYS_BEFORE
            .iter()
            .zip(flags)
            .map(|(p, flags)| {
                (
                    p.0,
                    p.1,
                    p.2,
                    flags,
                    if p.4.is_some() { peak } else { None },
                    p.5,
                )
            })
            .collect()
    };
    match category {
        // A volume wipe clears vol_up_early and vol_down (bits 4, 5) and peak_section; the score
        // is derived from every category.
        SignalCategory::Volume => (
            plays_with([2047 & !0x30, 5, 0], None),
            zeroed_score(),
            |e| SignalWipeReport {
                events_deleted: e,
                plays_cleared: 1,
                rollups_cleared: 3,
                ..SignalWipeReport::default()
            },
        ),
        SignalCategory::Speed => (
            plays_with([2047 & !0xC0, 5, 0], None),
            zeroed_score(),
            |e| SignalWipeReport {
                events_deleted: e,
                plays_cleared: 1,
                rollups_cleared: 3,
                ..SignalWipeReport::default()
            },
        ),
        // Outcome, replayed/seek flags and peak_section go; the identity of the play stays. The
        // rollup counters of outcomes go; fav_delta stays; the row of recording 3 had nothing else.
        SignalCategory::SeekSkipReplay => (
            vec![
                (0, 6, 0, 2047 & !0x0E, None, 1),
                (0, 6, 0, 1, None, 2),
                (0, 6, 0, 0, None, 3),
            ],
            vec![
                (1, 0, 0, 0, 0, 2, 0, 0.0),
                (2, 0, 0, 0, 0, 3, 0, 0.0),
                (4, 0, 0, 0, 0, 0, 0, 0.0),
            ],
            |e| SignalWipeReport {
                events_deleted: e,
                plays_cleared: 3,
                rollups_deleted: 1,
                rollups_cleared: 2,
                ..SignalWipeReport::default()
            },
        ),
        // The start identifies a play: its rows and the rollups go.
        SignalCategory::QueueLibrary => (vec![], vec![], |e| SignalWipeReport {
            events_deleted: e,
            plays_deleted: 3,
            rollups_deleted: 4,
            ..SignalWipeReport::default()
        }),
        SignalCategory::Browsing => (
            plays_with([2047 & !0x100, 5, 0], Some(3)),
            zeroed_score(),
            |e| SignalWipeReport {
                events_deleted: e,
                counter_rows_deleted: 3,
                plays_cleared: 1,
                rollups_cleared: 3,
                ..SignalWipeReport::default()
            },
        ),
        SignalCategory::OutputSound | SignalCategory::Session => {
            (plays_with([2047, 5, 0], Some(3)), zeroed_score(), |e| {
                SignalWipeReport {
                    events_deleted: e,
                    rollups_cleared: 3,
                    ..SignalWipeReport::default()
                }
            })
        }
    }
}

/// Several categories in one call do the union, a second identical wipe finds nothing, and no
/// category is a no-op.
#[test]
fn several_categories_at_once_and_a_repeated_wipe() {
    let mut fx = Fixture::new();
    let report = fx
        .wiper
        .wipe_signal_categories(&[
            SignalCategory::Volume,
            SignalCategory::Speed,
            SignalCategory::Volume,
        ])
        .unwrap();
    assert_eq!(report.events_deleted, 4);
    assert_eq!(report.plays_cleared, 1);
    let conn = fx.db.reader();
    assert_eq!(
        plays(&conn)[0].3,
        2047 & !0x30 & !0xC0,
        "vol_* and rate_* flags of the first play"
    );
    let once = fx.dump();

    let again = fx
        .wiper
        .wipe_signal_categories(&[SignalCategory::Volume, SignalCategory::Speed])
        .unwrap();
    assert_eq!(again, SignalWipeReport::default());
    assert_eq!(
        fx.wiper.wipe_signal_categories(&[]).unwrap(),
        SignalWipeReport::default()
    );
    assert_eq!(fx.dump(), once);
}

/// Wiping the seek/skip/replay category keeps what was played and when (the start is in another
/// category); the queue and library category removes the plays, so the outcome that is left would
/// belong to no recording.
#[test]
fn a_play_keeps_its_identity_unless_its_start_is_wiped() {
    let mut fx = Fixture::new();
    fx.wiper
        .wipe_signal_categories(&[SignalCategory::SeekSkipReplay])
        .unwrap();
    let conn = fx.db.reader();
    let kept: Vec<(i64, i64, i64, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT recording, started_ts, source IS NULL, context_key FROM play ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        kept,
        [
            (1, 5000, 0, 268_435_457),
            (2, 6000, 0, 0),
            (3, 7000, 1, 268_435_457)
        ]
    );
    // The reason is "unknown", not a skip: nothing downstream may read it as a dislike.
    assert_eq!(
        count(&conn, "SELECT count(*) FROM play WHERE end_reason = 6"),
        3
    );
    fx.wiper
        .wipe_signal_categories(&[SignalCategory::QueueLibrary])
        .unwrap();
    assert_eq!(count(&fx.db.reader(), "SELECT count(*) FROM play"), 0);
}

/// An event kind this build does not know belongs to no category: only wipe everything removes it.
#[test]
fn an_unknown_kind_survives_every_category_wipe_and_not_wipe_everything() {
    let mut fx = Fixture::new();
    fx.wiper
        .wipe_signal_categories(&SignalCategory::ALL)
        .unwrap();
    let conn = fx.db.reader();
    assert_eq!(event_kinds(&conn), [UNKNOWN_KIND]);
    // All categories at once also empties the counters and the derived tables of their columns.
    assert_eq!(count(&conn, "SELECT count(*) FROM ui_counter"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM play"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM rollup_monthly"), 0);
    // Context and the id tables are not signal data of a category.
    assert_eq!(count(&conn, "SELECT count(*) FROM context_snapshot"), 2);
    assert_eq!(count(&conn, "SELECT count(*) FROM session"), 1);

    fx.wiper.wipe_everything().unwrap();
    assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 0);
}

// ---------------------------------------------------------------- everything

/// Wipe everything empties every signal table, keeps the schema and the toggles, and reports what
/// it removed.
#[test]
fn wiping_everything_empties_the_signal_tables_and_keeps_the_settings() {
    let mut fx = Fixture::new();
    // A toggle the listener set: a wipe must not change it.
    let settings = PersistentSettings::open(
        &fx.db.path,
        tinwave_core::ManualClock::new(tinwave_core::UtcMillis::new(T0)),
    )
    .unwrap();
    settings
        .set_category(SignalCategory::Browsing, false)
        .unwrap();
    let mask_before = settings.settings().mask();
    let before = fx.dump();
    let events = i64::try_from(SignalKind::events().count() + 1).unwrap();

    let report = fx.wiper.wipe_everything().unwrap();

    let conn = fx.db.reader();
    let after = fx.dump();
    for name in &TABLES[..10] {
        assert!(table(&after, name).is_empty(), "{name} is not empty");
    }
    assert_eq!(
        table(&after, "signal_settings"),
        table(&before, "signal_settings")
    );
    assert_eq!(
        table(&after, "schema_migration"),
        table(&before, "schema_migration")
    );
    assert_eq!(
        report,
        SignalWipeReport {
            events_deleted: usize::try_from(events).unwrap(),
            counter_rows_deleted: 3,
            plays_deleted: 3,
            rollups_deleted: 4,
            snapshots_deleted: 2,
            sessions_deleted: 1,
            dictionary_rows_deleted: 1 + 3 + 1 + 1,
            ..SignalWipeReport::default()
        }
    );
    let reopened = PersistentSettings::open(
        &fx.db.path,
        tinwave_core::ManualClock::new(tinwave_core::UtcMillis::new(T0)),
    )
    .unwrap();
    assert_eq!(reopened.settings().mask(), mask_before);
    // The schema is intact: the tables can be filled again.
    populate(&conn);
    assert_eq!(count(&conn, "SELECT count(*) FROM play"), 3);
    // A wipe of an empty database finds nothing.
    let mut empty = Fixture::new();
    empty.wiper.wipe_everything().unwrap();
    assert_eq!(
        empty.wiper.wipe_everything().unwrap(),
        SignalWipeReport::default()
    );
}

/// The writer keeps its dictionary and session caches across batches. After a wipe of everything,
/// the next batch must not trust them: the integers they hold point at deleted rows (or, since an
/// emptied table hands out its integers again, at another recording).
#[test]
fn a_store_with_warm_caches_writes_consistent_rows_after_a_wipe_of_everything() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut wiper = SignalWiper::open(&db.path).unwrap();
    let mut ids = Ids::new(71);
    let (device, session, first, second) = (
        ids.device(),
        ids.session(),
        ids.recording(),
        ids.recording(),
    );
    store
        .write_batch(&[favorited(device, session, first, 0)], &[])
        .unwrap();
    assert!(store.cached_entries() >= 3, "the caches are warm");

    wiper.wipe_everything().unwrap();
    // A different recording first: with a stale cache the new row would take the integer the
    // cache still maps to `first`.
    store
        .write_batch(&[favorited(device, session, second, 1)], &[])
        .unwrap();
    store
        .write_batch(&[favorited(device, session, first, 2)], &[])
        .unwrap();

    let conn = db.reader();
    for (seq, recording) in [(1i64, second), (2, first)] {
        let uuid: Vec<u8> = conn
            .query_row(
                "SELECT r.uuid FROM signal_event e JOIN dict_recording r ON r.id = e.recording \
                 WHERE e.seq = ?1",
                [seq],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(uuid, recording.as_bytes(), "seq {seq}");
    }
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM signal_event e \
             JOIN session s ON s.id = e.session JOIN dict_device d ON d.id = s.device"
        ),
        2,
        "every event has its session and device row"
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM dict_recording"), 2);
}

/// The real writer thread, fed by the bus, runs while everything is wiped: no commit fails, no
/// event is lost (each is either wiped or stored), and what was written after the wipe is
/// consistent.
#[test]
fn the_writer_thread_keeps_working_while_everything_is_wiped() {
    let db = TempDb::new();
    let store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(72);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let mut cfg = BusConfig::new(device);
    cfg.first_seq = store.first_seq(device).unwrap();
    let (bus, drain) =
        SignalBus::channel(cfg, ids.clock.clone(), Arc::new(SignalSettings::new())).unwrap();
    let writer = SignalWriter::spawn(
        store,
        drain,
        WriterConfig {
            flush_interval: Duration::from_millis(5),
            flush_events: 4,
            stop_poll: Duration::from_millis(5),
        },
    )
    .unwrap();
    let mut wiper = SignalWiper::open(&db.path).unwrap();

    const EVENTS: u64 = 600;
    let report = std::thread::scope(|scope| {
        let wipe = scope.spawn(|| {
            let conn = db.reader();
            // Wipe once the writer has stored a good part of the events.
            while count(&conn, "SELECT count(*) FROM signal_event") < 100 {
                std::thread::sleep(Duration::from_millis(1));
            }
            wiper.wipe_everything().unwrap()
        });
        for _ in 0..EVENTS {
            let _ = bus
                .emit_with(SignalKind::Favorited, |st| {
                    favorited(device, session, rec, st.id.seq)
                })
                .unwrap();
            std::thread::sleep(Duration::from_micros(500));
        }
        wipe.join().unwrap()
    });
    let accepted = bus.queued_total();
    drop(bus);
    let stats = writer.shutdown().unwrap();

    assert_eq!(accepted, EVENTS);
    assert_eq!(stats.failed_commits, 0, "a batch had to be retried");
    assert_eq!(stats.events_lost, 0);
    assert_eq!(stats.events_inserted, accepted);
    let conn = db.reader();
    let remaining = count(&conn, "SELECT count(*) FROM signal_event");
    assert_eq!(
        i64::try_from(report.events_deleted).unwrap() + remaining,
        i64::try_from(EVENTS).unwrap(),
        "every event is either wiped or stored"
    );
    assert!(report.events_deleted >= 100);
    assert!(remaining > 0, "events written after the wipe are stored");
    // What was stored after the wipe points at rows that exist and name the right recording.
    let uuids: Vec<Vec<u8>> = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT r.uuid FROM signal_event e \
                 JOIN dict_recording r ON r.id = e.recording",
            )
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(uuids, [rec.as_bytes().to_vec()]);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM signal_event e JOIN session s ON s.id = e.session \
             JOIN dict_device d ON d.id = s.device"
        ),
        remaining,
        "every stored event has its session and device row"
    );
}

// ---------------------------------------------------------------- atomic

fn inject_failure(conn: &Connection, on: &str) {
    conn.execute_batch(&format!(
        "CREATE TRIGGER inject_failure BEFORE {on} BEGIN SELECT RAISE(ABORT, 'injected failure'); END;"
    ))
    .unwrap();
}

/// A failure in the last statement of a wipe leaves every table as it was.
#[test]
fn a_failed_wipe_changes_nothing_and_works_once_the_fault_is_gone() {
    // (what to wipe, where the failure is injected: the last table the wipe changes)
    let cases: [(&[SignalCategory], &str); 4] = [
        // Rollups are cleared last, after the events and the play columns.
        (
            &[SignalCategory::SeekSkipReplay],
            "UPDATE ON rollup_monthly",
        ),
        (
            &[SignalCategory::Volume, SignalCategory::Browsing],
            "UPDATE ON rollup_monthly",
        ),
        // The queue and library wipe deletes the rollups last.
        (&[SignalCategory::QueueLibrary], "DELETE ON rollup_monthly"),
        // Everything: the first dictionary emptied last.
        (&[], "DELETE ON dict_device"),
    ];
    for (categories, failure) in cases {
        let mut fx = Fixture::new();
        let before = fx.dump();
        let injector = fx.db.reader();
        inject_failure(&injector, failure);

        let error = if categories.is_empty() {
            fx.wiper.wipe_everything().unwrap_err()
        } else {
            fx.wiper.wipe_signal_categories(categories).unwrap_err()
        };
        assert!(
            error.chain().contains("injected failure"),
            "{}",
            error.chain()
        );
        assert_eq!(fx.dump(), before, "{categories:?}: a table changed");

        injector
            .execute_batch("DROP TRIGGER inject_failure;")
            .unwrap();
        let report = if categories.is_empty() {
            fx.wiper.wipe_everything().unwrap()
        } else {
            fx.wiper.wipe_signal_categories(categories).unwrap()
        };
        assert!(report.events_deleted > 0, "{categories:?}");
        assert_ne!(fx.dump(), before, "{categories:?}");
    }
}

// ---------------------------------------------------------------- contention

/// Inserts a reference listener's tables (3 years of plays, the hot tier of events, counters,
/// snapshots, a large dictionary) in one transaction.
fn populate_reference(conn: &Connection) {
    const PLAYS: i64 = 131_400;
    const EVENTS: i64 = 40_000;
    conn.execute_batch(
        "BEGIN;
         INSERT INTO dict_device (id, uuid) VALUES (1, x'01');
         INSERT INTO session (id, uuid, device, started_ts, tz_offset_min) VALUES (1, x'0f', 1, 1000, 60);",
    )
    .unwrap();
    conn.execute(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 50000)
         INSERT INTO dict_recording (id, uuid) SELECT x, randomblob(16) FROM c",
        [],
    )
    .unwrap();
    conn.execute(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?1)
         INSERT INTO play (device, session, recording, source, started_ts, listened_ds, end_reason, \
                           end_frac, flags, peak_section, context_key)
         SELECT 1, 1, x % 50000 + 1, NULL, x * 1000, 1800, x % 4, 100, x % 8, NULL, x % 4000 FROM c",
        [PLAYS],
    )
    .unwrap();
    conn.execute(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?1)
         INSERT INTO signal_event (device, seq, session, ts_off, kind, recording, ctx)
         SELECT 1, x, 1, x, CASE x % 3 WHEN 0 THEN 1 WHEN 1 THEN 33 ELSE 54 END, x % 50000 + 1, x % 400 + 1 FROM c",
        [EVENTS],
    )
    .unwrap();
    conn.execute(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 20000)
         INSERT INTO ui_counter (day, kind, entity_kind, entity, count) SELECT 20260101 + x % 90, 65 + x % 3, 0, x, 1 FROM c",
        [],
    )
    .unwrap();
    conn.execute(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 400)
         INSERT INTO context_snapshot (id, hash, key, fields) SELECT x, x * 7919, 0, x'00' FROM c",
        [],
    )
    .unwrap();
    conn.execute(
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 20000)
         INSERT INTO rollup_monthly (month, recording, plays, completes, skips_early, skips_late, \
                                     fav_delta, listened_s, score_sum)
         SELECT 202601 + x % 12, x, 3, 1, 1, 1, 0, 600, 1.5 FROM c",
        [],
    )
    .unwrap();
    conn.execute_batch("COMMIT;").unwrap();
}

/// What the writer sends while a wipe runs: a play that starts and is skipped.
fn play_batch(ids: &mut Ids) -> Vec<tinwave_signals::SignalEvent> {
    use tinwave_core::{SignalEventId, UtcMillis};
    use tinwave_signals::{
        Fraction, InputSource, PlayOrigin, PlayStartedPayload, QueueOrigin, Section, SignalEvent,
        SignalPayload, SkipPayload,
    };
    let (device, session, rec, source) =
        (ids.device(), ids.session(), ids.recording(), ids.source());
    let make = |seq: u64, at: i64, payload: SignalPayload| SignalEvent {
        id: SignalEventId::new(device, seq),
        ts: UtcMillis::new(T0 + at),
        session,
        recording: Some(rec),
        source: Some(source),
        pos_ds: None,
        payload,
        ctx: None,
    };
    vec![
        make(
            0,
            0,
            SignalPayload::PlayStarted(PlayStartedPayload {
                origin: PlayOrigin::Queue,
                input: InputSource::Auto,
            }),
        ),
        make(
            1,
            90_000,
            SignalPayload::Skipped(SkipPayload {
                fraction: Fraction::from_raw(100),
                input: InputSource::Mouse,
                section: Section::UNKNOWN,
                queue_origin: QueueOrigin::Manual,
            }),
        ),
    ]
}

/// A batch the writer sends while the reference-size tables are wiped is stored, within the busy
/// timeout, for a category wipe (which rewrites the 131 400 play rows) and for wipe everything.
/// The probe must have seen the wipe's lock, so the contention really happened.
#[test]
fn a_write_batch_during_a_wipe_of_the_reference_tables_is_stored() {
    for everything in [false, true] {
        let db = TempDb::new();
        let mut store = SignalStore::open(&db.path).unwrap();
        populate_reference(&db.reader());
        let mut wiper = SignalWiper::open(&db.path).unwrap();
        let mut ids = Ids::new(73);
        let events = play_batch(&mut ids);
        // Warm the caches with the same session and recording the batch uses: after the wipe of
        // everything they are stale.
        store.write_batch(&events[..1], &[]).unwrap();
        // The wipe holds the lock for at least the burn of this trigger.
        burn_in_trigger(
            &db.reader(),
            if everything {
                "DELETE ON dict_device"
            } else {
                "UPDATE ON play"
            },
        );
        let rest = events[1..].to_vec();

        let (report, (batch, waited), lock_seen) = during_a_wipe(
            &db,
            || {
                if everything {
                    wiper.wipe_everything().unwrap()
                } else {
                    wiper
                        .wipe_signal_categories(&[SignalCategory::SeekSkipReplay])
                        .unwrap()
                }
            },
            || {
                let started = std::time::Instant::now();
                let batch = store.write_batch(&rest, &[]);
                (batch, started.elapsed())
            },
        );
        assert!(
            lock_seen,
            "everything={everything}: the probe never saw the wipe's lock"
        );
        let batch = batch.unwrap_or_else(|e| {
            panic!(
                "everything={everything}: the batch failed after {waited:?}: {}",
                e.chain()
            )
        });
        assert_eq!(batch.events_inserted, 1, "everything={everything}");

        let conn = db.reader();
        if everything {
            assert_eq!(report.plays_deleted, 131_400);
            // The PlayStarted was wiped: the skip has no start to pair with.
            assert_eq!(batch.plays_unpaired, 1);
            assert_eq!(count(&conn, "SELECT count(*) FROM play"), 0);
            assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 1);
            // The writer's caches were stale after the wipe: the row it wrote must still point at
            // dictionary, session and device rows that exist.
            assert_eq!(
                count(
                    &conn,
                    "SELECT count(*) FROM signal_event e JOIN session s ON s.id = e.session \
                     JOIN dict_device d ON d.id = s.device \
                     JOIN dict_recording r ON r.id = e.recording \
                     JOIN dict_source o ON o.id = e.source"
                ),
                1
            );
        } else {
            assert_eq!(report.plays_cleared, 131_400);
            assert_eq!(
                batch.plays_written, 1,
                "the new play is stored after the wipe"
            );
            assert_eq!(count(&conn, "SELECT count(*) FROM play"), 131_401);
            assert_eq!(
                count(&conn, "SELECT count(*) FROM play WHERE end_reason = 6"),
                131_400,
                "the wiped plays have no outcome; the new one keeps its skip"
            );
        }
    }
}

// ---------------------------------------------------------------- deleted means gone

const MARKER: &[u8] = b"TINWAVE-WIPE-MARKER-0123456789abcdef-xyz";

fn file_holds_marker(db: &TempDb) -> bool {
    let wal = {
        let mut p = db.path.as_os_str().to_owned();
        p.push("-wal");
        std::path::PathBuf::from(p)
    };
    [db.path.clone(), wal].iter().any(|path| {
        std::fs::read(path)
            .map(|bytes| bytes.windows(MARKER.len()).any(|w| w == MARKER))
            .unwrap_or(false)
    })
}

fn checkpoint(conn: &Connection) {
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy, 0, "the checkpoint was blocked");
}

/// Deleted content is zeroed in the file, not left in the free pages (and not in the WAL once it
/// is checkpointed): a wipe that only unlinked the rows would leave the data readable.
#[test]
fn wiped_content_is_not_left_in_the_database_file() {
    for everything in [false, true] {
        let db = TempDb::new();
        SignalStore::open(&db.path).unwrap();
        let conn = db.reader();
        conn.execute_batch(
            "INSERT INTO dict_device (id, uuid) VALUES (1, x'01');
             INSERT INTO session (id, uuid, device, started_ts, tz_offset_min) VALUES (1, x'0f', 1, 1000, 0);",
        )
        .unwrap();
        // A payload of a volume event, and a context snapshot blob.
        conn.execute(
            "INSERT INTO signal_event (device, seq, session, ts_off, kind, payload) \
             VALUES (1, 1, 1, 0, ?1, ?2)",
            params![SignalKind::VolumeChanged.discriminant(), MARKER],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO context_snapshot (id, hash, key, fields) VALUES (1, 1, 0, ?1)",
            [MARKER],
        )
        .unwrap();
        checkpoint(&conn);
        assert!(
            file_holds_marker(&db),
            "the fixture wrote the marker to the file"
        );

        let mut wiper = SignalWiper::open(&db.path).unwrap();
        if everything {
            wiper.wipe_everything().unwrap();
        } else {
            wiper
                .wipe_signal_categories(&[SignalCategory::Volume])
                .unwrap();
        }
        checkpoint(&conn);
        // The snapshot is not part of a category wipe: the volume payload is the only marker left
        // there, in the snapshot's blob. Check what the wipe was asked to remove.
        let still_there = file_holds_marker(&db);
        if everything {
            assert!(!still_there, "wiped data is still readable in the file");
        } else {
            // One marker was removed (the event payload), the snapshot's copy stays: the file
            // holds exactly one.
            let bytes = std::fs::read(&db.path).unwrap();
            let copies = bytes.windows(MARKER.len()).filter(|w| *w == MARKER).count();
            assert_eq!(
                copies, 1,
                "the event payload copy is zeroed, the snapshot's stays"
            );
        }
    }
}
