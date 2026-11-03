//! Wiping a signal category, a privacy tier and everything (T-0061, T-0064). Every test drives the
//! real `SignalWiper` on a SQLite file; failures are injected from a second connection with a
//! trigger (as in `context_wipe.rs`), and the contention tests run the real writer path while the
//! wipe holds the write lock.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{
    burn_in_trigger, count, during_a_wipe, favorited, statements_of, write_batch_after_the_wipe,
    write_batch_when_unlocked, Ids, TempDb, AFTER_WIPE_SLACK, BUSY_TIMEOUT, T0,
};
use rusqlite::types::Value;
use rusqlite::{params, Connection};
use tinwave_core::Tier;
use tinwave_db::{
    ContextCategory, PersistentSettings, SignalStore, SignalWipeReport, SignalWiper, SignalWriter,
    WipeReport, WriterConfig,
};
use tinwave_signals::{
    ActivityGuess, BusConfig, ContextKey, Daypart, OutputClass, PlaceSlot, SignalBus,
    SignalCategory, SignalKind, SignalSettings, TemperatureBand, WeatherCondition, WeekdayType,
};

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
        // rollup counters of outcomes go; `plays` (the starts) and fav_delta stay, so no row is
        // left with only zeros (T-0065: the play count is kept, like the play rows).
        SignalCategory::SeekSkipReplay => (
            vec![
                (0, 6, 0, 2047 & !0x0E, None, 1),
                (0, 6, 0, 1, None, 2),
                (0, 6, 0, 0, None, 3),
            ],
            vec![
                (1, 10, 0, 0, 0, 2, 0, 0.0),
                (2, 0, 0, 0, 0, 3, 0, 0.0),
                (3, 5, 0, 0, 0, 0, 0, 0.0),
                (4, 0, 0, 0, 0, 0, 0, 0.0),
            ],
            |e| SignalWipeReport {
                events_deleted: e,
                plays_cleared: 3,
                rollups_cleared: 3,
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
    // The monthly play count is the same fact: the seek wipe keeps it (T-0065), only its
    // outcome counters go.
    assert_eq!(
        ints::<i64>(&conn, "SELECT plays FROM rollup_monthly ORDER BY recording"),
        [10, 0, 5, 0]
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM rollup_monthly WHERE completes != 0 OR skips_early != 0 \
             OR skips_late != 0 OR listened_s != 0"
        ),
        0
    );
    fx.wiper
        .wipe_signal_categories(&[SignalCategory::QueueLibrary])
        .unwrap();
    assert_eq!(count(&fx.db.reader(), "SELECT count(*) FROM play"), 0);
    assert_eq!(
        count(&fx.db.reader(), "SELECT count(*) FROM rollup_monthly"),
        0,
        "the queue and library wipe clears the play count with its rows"
    );
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

// ---------------------------------------------------------------- per tier

/// The event kinds of tier T1 (`WindowState`, `OutputDeviceChanged`), written out from the catalog.
/// Every other stored kind is T0; no kind is T2 or T3.
const T1_EVENT_KINDS: [u8; 2] = [69, 81];

/// Each tier deletes exactly its own events and counters and the columns derived from them, and
/// nothing else in any table.
#[test]
fn wiping_a_tier_deletes_exactly_its_events_and_the_data_derived_from_it() {
    for tier in Tier::ALL {
        let mut fx = Fixture::new();
        let before = fx.dump();

        let report = fx.wiper.wipe_signal_tiers(&[tier]).unwrap();

        let conn = fx.db.reader();
        let after = dump(&conn);
        match tier {
            // 36 of the 38 event kinds and all the counters: every T0 signal, so the play starts
            // too. The two T1 events and the unknown kind stay.
            Tier::T0 => {
                assert_eq!(event_kinds(&conn), [69, 81, UNKNOWN_KIND], "T0 events");
                assert_eq!(count(&conn, "SELECT count(*) FROM ui_counter"), 0);
                assert_eq!(count(&conn, "SELECT count(*) FROM play"), 0);
                assert_eq!(count(&conn, "SELECT count(*) FROM rollup_monthly"), 0);
                assert_eq!(
                    report,
                    SignalWipeReport {
                        events_deleted: 36,
                        counter_rows_deleted: 3,
                        plays_deleted: 3,
                        rollups_deleted: 4,
                        ..SignalWipeReport::default()
                    }
                );
            }
            // The two T1 events go. No play column is derived from them; the score, a blend of
            // every signal, is reset like in every other wipe.
            Tier::T1 => {
                let mut expected: Vec<u8> = SignalKind::events()
                    .map(|k| k.discriminant())
                    .filter(|k| !T1_EVENT_KINDS.contains(k))
                    .chain([UNKNOWN_KIND])
                    .collect();
                expected.sort_unstable();
                assert_eq!(event_kinds(&conn), expected, "T1 events");
                assert_eq!(count(&conn, "SELECT count(*) FROM ui_counter"), 3);
                assert_eq!(plays(&conn), PLAYS_BEFORE);
                assert_eq!(
                    rollups(&conn),
                    ROLLUPS_BEFORE.map(|r| (r.0, r.1, r.2, r.3, r.4, r.5, r.6, 0.0))
                );
                assert_eq!(
                    report,
                    SignalWipeReport {
                        events_deleted: 2,
                        rollups_cleared: 3,
                        ..SignalWipeReport::default()
                    }
                );
            }
            // No signal kind has these tiers: nothing is wiped, in any table.
            Tier::T2 | Tier::T3 => {
                assert_eq!(after, before, "{tier:?}");
                assert_eq!(report, SignalWipeReport::default());
            }
        }
        // Context, sessions, dictionaries and settings are not signals of a tier.
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
                "{tier:?}: {name}"
            );
        }
    }
}

/// Several tiers do the union (all tiers is what all categories are), a repeated wipe finds
/// nothing, and no tier is a no-op.
#[test]
fn several_tiers_at_once_and_a_repeated_wipe() {
    let mut by_tiers = Fixture::new();
    let report = by_tiers
        .wiper
        .wipe_signal_tiers(&[Tier::T1, Tier::T0, Tier::T1])
        .unwrap();
    assert_eq!(report.events_deleted, 38);
    assert_eq!(event_kinds(&by_tiers.db.reader()), [UNKNOWN_KIND]);
    let once = by_tiers.dump();
    assert_eq!(
        by_tiers.wiper.wipe_signal_tiers(&Tier::ALL).unwrap(),
        SignalWipeReport::default()
    );
    assert_eq!(
        by_tiers.wiper.wipe_signal_tiers(&[]).unwrap(),
        SignalWipeReport::default()
    );
    assert_eq!(by_tiers.dump(), once);

    // Every tier wiped is every category wiped: the same rows are left.
    let mut by_categories = Fixture::new();
    by_categories
        .wiper
        .wipe_signal_categories(&SignalCategory::ALL)
        .unwrap();
    assert_eq!(by_categories.dump(), once);
}

/// A key with the listed fields known (daypart, weekday, condition, temperature, device,
/// activity, place), from the typed builders and not from the code under test.
fn key_with(keep: [bool; 7]) -> u32 {
    let mut key = ContextKey::UNKNOWN;
    if keep[0] {
        key = key.with_daypart(Daypart::Evening);
    }
    if keep[1] {
        key = key.with_weekday_type(WeekdayType::Weekend);
    }
    if keep[2] {
        key = key.with_condition(WeatherCondition::Rain);
    }
    if keep[3] {
        key = key.with_temperature(TemperatureBand::Mild);
    }
    if keep[4] {
        key = key.with_device_class(OutputClass::Bluetooth);
    }
    if keep[5] {
        key = key.with_activity(ActivityGuess::Leisure);
    }
    if keep[6] {
        key = key.with_place(PlaceSlot::new(5).unwrap());
    }
    key.raw()
}

fn context_keys(conn: &Connection) -> (Vec<u32>, Vec<u32>) {
    (
        ints(conn, "SELECT context_key FROM play ORDER BY id"),
        ints(conn, "SELECT key FROM context_snapshot ORDER BY id"),
    )
}

/// "Wipe tier T2" is one call: the tier's signal events and derived data and the tier's context
/// fields go together. T2 has no signal kind, so only the context goes (activity and place); T1
/// takes its two events, the score, and the clock and output fields.
#[test]
fn a_tier_wipe_is_one_call_for_the_signals_and_the_context_of_the_tier() {
    let mut fx = Fixture::new();
    let conn = fx.db.reader();
    let full = key_with([true; 7]);
    conn.execute("UPDATE play SET context_key = ?1", [full])
        .unwrap();
    conn.execute("UPDATE context_snapshot SET key = ?1", [full])
        .unwrap();
    let before = fx.dump();

    // T2: activity (app) and place.
    let (signals, context) = fx.wiper.wipe_tiers(&[Tier::T2]).unwrap();
    assert_eq!(signals, SignalWipeReport::default());
    assert_eq!(
        context,
        WipeReport {
            plays_rewritten: 3,
            snapshots_rewritten: 2
        }
    );
    let t2 = key_with([true, true, true, true, true, false, false]);
    assert_eq!(context_keys(&conn), (vec![t2; 3], vec![t2; 2]));
    let after = fx.dump();
    for name in [
        "signal_event",
        "ui_counter",
        "rollup_monthly",
        "session",
        "dict_device",
    ] {
        assert_eq!(table(&after, name), table(&before, name), "{name}");
    }

    // T1 and T3 together: the signals of T1, the clock, device and weather fields. Every field of
    // the key belongs to T1, T2 or T3, so nothing is left.
    let (signals, context) = fx.wiper.wipe_tiers(&[Tier::T3, Tier::T1]).unwrap();
    assert_eq!(
        signals,
        SignalWipeReport {
            events_deleted: 2,
            rollups_cleared: 3,
            ..SignalWipeReport::default()
        }
    );
    assert_eq!(context.plays_rewritten, 3);
    assert_eq!(context.snapshots_rewritten, 2);
    assert_eq!(context_keys(&conn), (vec![0; 3], vec![0; 2]));
    assert_eq!(
        event_kinds(&conn).len(),
        37,
        "36 T0 events and the unknown kind"
    );

    // T0 has no context: its wipe is the signals only, and the context columns stay as they are.
    // (The play rows are deleted with the play starts, so the keys are checked on the snapshots.)
    conn.execute("UPDATE context_snapshot SET key = ?1", [full])
        .unwrap();
    let (signals, context) = fx.wiper.wipe_tiers(&[Tier::T0]).unwrap();
    assert_eq!(signals.events_deleted, 36);
    assert_eq!(context, WipeReport::default());
    assert_eq!(context_keys(&conn).1, vec![full; 2]);

    assert_eq!(
        fx.wiper.wipe_tiers(&[]).unwrap(),
        (SignalWipeReport::default(), WipeReport::default())
    );
}

/// The wipe of a tier is atomic across its signal part and its context part: a failure in the
/// context rewrite (the last thing the wipe does) leaves the events, the derived columns and the
/// keys as they were, and the wipe works once the fault is gone. The signal-only wipe fails the
/// same way.
#[test]
fn a_failed_tier_wipe_changes_nothing_and_works_once_the_fault_is_gone() {
    for combined in [true, false] {
        let mut fx = Fixture::new();
        fx.db
            .reader()
            .execute(
                "UPDATE context_snapshot SET key = ?1",
                [key_with([true; 7])],
            )
            .unwrap();
        let before = fx.dump();
        let injector = fx.db.reader();
        // The signal part runs first (it ends with the rollups); the context part ends with the
        // snapshots.
        inject_failure(
            &injector,
            if combined {
                "UPDATE ON context_snapshot"
            } else {
                "UPDATE ON rollup_monthly"
            },
        );

        let error = if combined {
            fx.wiper.wipe_tiers(&[Tier::T1]).unwrap_err()
        } else {
            fx.wiper.wipe_signal_tiers(&[Tier::T1]).unwrap_err()
        };
        assert!(
            error.chain().contains("injected failure"),
            "{}",
            error.chain()
        );
        assert_eq!(fx.dump(), before, "combined={combined}: a table changed");

        injector
            .execute_batch("DROP TRIGGER inject_failure;")
            .unwrap();
        let events = if combined {
            fx.wiper.wipe_tiers(&[Tier::T1]).unwrap().0.events_deleted
        } else {
            fx.wiper
                .wipe_signal_tiers(&[Tier::T1])
                .unwrap()
                .events_deleted
        };
        assert_eq!(events, 2, "combined={combined}");
        assert_ne!(fx.dump(), before);
    }
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

/// The real writer thread, fed by the bus, runs while everything is wiped: no event is lost (each is
/// either wiped or stored), what was written after the wipe is consistent, and a commit fails only
/// if the wipe really held the lock for longer than the busy timeout (the writer retries it).
///
/// Nothing here is a count of emitted events or a fixed sleep that a slow machine can outrun: the
/// emitter waits on conditions (events stored, the wipe returned) with budgets of
/// [`LOAD_BUDGET`], and the wipe is held open by a burn trigger so the writer's batches do queue
/// behind it.
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
    let emit = || {
        let _ = bus
            .emit_with(SignalKind::Favorited, |st| {
                favorited(device, session, rec, st.id.seq)
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(1));
    };
    let deadline = Instant::now() + LOAD_BUDGET;
    let within_budget = |what: &str| {
        assert!(Instant::now() < deadline, "{what}: out of budget");
    };

    // Let the writer store a good part of the events first.
    let conn = db.reader();
    while count(&conn, "SELECT count(*) FROM signal_event") < 100 {
        emit();
        within_budget("waiting for 100 stored events");
    }
    // The wipe holds the write lock for at least the burn of this trigger.
    burn_in_trigger(&conn, "DELETE ON dict_device");

    let ((report, wipe_time), (), lock_seen) = during_a_wipe(
        &db,
        || {
            let started = Instant::now();
            let report = wiper.wipe_everything().unwrap();
            (report, started.elapsed())
        },
        |wipe_end| {
            // Events keep coming while the wipe runs, and after it: 50 events that cannot have
            // been wiped.
            while wipe_end.get().is_none() {
                emit();
                within_budget("waiting for the wipe to return");
            }
            for _ in 0..50 {
                emit();
            }
        },
    );
    assert!(
        lock_seen,
        "the probe never saw the wipe's lock: the writer was not contended"
    );
    let accepted = bus.queued_total();
    drop(bus);
    let stats = writer.shutdown().unwrap();
    eprintln!("wipe everything {wipe_time:?}, writer {stats:?}");

    assert_eq!(stats.events_lost, 0);
    assert_eq!(stats.events_inserted, accepted);
    // A commit may only fail, and be retried, when the wipe held the lock past the busy timeout.
    assert!(
        stats.failed_commits == 0 || wipe_time > BUSY_TIMEOUT,
        "{} commits failed although the wipe took only {wipe_time:?}",
        stats.failed_commits
    );
    let conn = db.reader();
    let remaining = count(&conn, "SELECT count(*) FROM signal_event");
    assert_eq!(
        i64::try_from(report.events_deleted).unwrap() + remaining,
        i64::try_from(accepted).unwrap(),
        "every event is either wiped or stored"
    );
    assert!(report.events_deleted >= 100);
    assert!(
        remaining >= 50,
        "events written after the wipe are stored ({remaining})"
    );
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

// ---------------------------------------------------------------- the wipe epoch

/// The wipe epoch the writer compares between batches (`PRAGMA user_version`).
fn epoch(conn: &Connection) -> i64 {
    count(conn, "PRAGMA user_version")
}

/// Only a wipe of everything moves the epoch (it deletes the rows the writer's caches point at);
/// a category wipe, a context wipe and a settings toggle do not, and each wipe of everything moves
/// it again.
#[test]
fn only_a_wipe_of_everything_moves_the_wipe_epoch() {
    let mut fx = Fixture::new();
    let conn = fx.db.reader();
    assert_eq!(epoch(&conn), 0);
    fx.wiper
        .wipe_signal_categories(&SignalCategory::ALL)
        .unwrap();
    fx.wiper
        .wipe_context(&tinwave_db::ContextCategory::ALL)
        .unwrap();
    PersistentSettings::open(
        &fx.db.path,
        tinwave_core::ManualClock::new(tinwave_core::UtcMillis::new(T0)),
    )
    .unwrap()
    .set_category(SignalCategory::Browsing, false)
    .unwrap();
    assert_eq!(epoch(&conn), 0, "nothing the caches point at was deleted");
    fx.wiper.wipe_everything().unwrap();
    assert_eq!(epoch(&conn), 1);
    fx.wiper.wipe_everything().unwrap();
    assert_eq!(epoch(&conn), 2);
}

/// The writer keeps its caches across commits of other connections that are not a wipe (the
/// context hub writes snapshots, the settings are toggled): dropping them before almost every
/// batch cost about 30 % of the writer's CPU in the 90-day growth run. The proof that the cache is
/// still used: a dictionary row another connection changed under it is not looked up again.
/// (Real code never edits a dictionary row; a wipe is the only thing that deletes them, and it
/// moves the epoch, see `a_store_with_warm_caches_writes_consistent_rows_after_a_wipe_of_everything`.)
#[test]
fn a_commit_of_another_connection_that_is_not_a_wipe_keeps_the_writer_caches() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(74);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    store
        .write_batch(&[favorited(device, session, rec, 0)], &[])
        .unwrap();
    let cached = store.cached_entries();
    assert!(cached >= 3, "the caches are warm");

    // Commits of other connections: a settings toggle, a context snapshot, a category wipe.
    let conn = db.reader();
    PersistentSettings::open(
        &db.path,
        tinwave_core::ManualClock::new(tinwave_core::UtcMillis::new(T0)),
    )
    .unwrap()
    .set_category(SignalCategory::Browsing, false)
    .unwrap();
    conn.execute(
        "INSERT INTO context_snapshot (id, hash, key, fields) VALUES (1, 1, 0, x'00')",
        [],
    )
    .unwrap();
    SignalWiper::open(&db.path)
        .unwrap()
        .wipe_signal_categories(&[SignalCategory::Volume])
        .unwrap();
    // The recording's dictionary row now holds another UUID, as if it had been replaced.
    conn.execute(
        "UPDATE dict_recording SET uuid = x'ffffffffffffffffffffffffffffffff'",
        [],
    )
    .unwrap();

    store
        .write_batch(&[favorited(device, session, rec, 1)], &[])
        .unwrap();
    assert_eq!(
        count(&conn, "SELECT count(*) FROM dict_recording"),
        1,
        "the cached id was used, not looked up (a lookup would have inserted a second row)"
    );
    assert_eq!(store.cached_entries(), cached);
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
        assert_eq!(
            epoch(&injector),
            0,
            "{categories:?}: the failed wipe moved the wipe epoch, so the writer would drop its caches for a wipe that did not happen"
        );

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
        assert_eq!(epoch(&injector), i64::from(categories.is_empty()));
    }
}

// ---------------------------------------------------------------- cost

/// 3 000 more rows in every table a wipe reaches, on top of [`populate`]: distinct context keys,
/// events of 90 kinds, and play and rollup rows that every category and tier wipe changes. A
/// statement per row (or per distinct key, or per kind) would make a wipe of this database run
/// thousands of statements more than a wipe of the three-row one.
fn populate_many(conn: &Connection) {
    const ROWS: &str =
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 3000)";
    conn.execute_batch(&format!(
        "BEGIN;
         {ROWS} INSERT INTO dict_recording (id, uuid) SELECT 100 + x, randomblob(16) FROM c;
         {ROWS} INSERT INTO dict_source (id, uuid) SELECT 100 + x, randomblob(16) FROM c;
         {ROWS} INSERT INTO dict_entity (id, kind, uuid) SELECT 100 + x, 1, randomblob(16) FROM c;
         {ROWS} INSERT INTO session (id, uuid, device, started_ts, tz_offset_min)
                SELECT 100 + x, randomblob(16), 1, 1000, 60 FROM c;
         {ROWS} INSERT INTO context_snapshot (id, hash, key, fields)
                SELECT 100 + x, 1000 + x * 7919, x % 4000, x'00' FROM c;
         {ROWS} INSERT INTO signal_event (device, seq, session, ts_off, kind, recording, ctx)
                SELECT 1, 10000 + x, 1, x, x % 90 + 1, 1 + x % 3, 1 FROM c;
         {ROWS} INSERT INTO ui_counter (day, kind, entity_kind, entity, count)
                SELECT 20261101, 65 + x % 3, 0, 100 + x, 1 FROM c;
         {ROWS} INSERT INTO play (device, session, recording, source, started_ts, listened_ds,
                                  end_reason, end_frac, flags, peak_section, context_key)
                SELECT 1, 1, 1 + x % 3, NULL, 10000 + x, 100 + x, x % 4, 100, x % 2048, x % 4,
                       x % 4000 FROM c;
         {ROWS} INSERT INTO rollup_monthly (month, recording, plays, completes, skips_early,
                                            skips_late, fav_delta, listened_s, score_sum)
                SELECT 202601 + x % 12, 100 + x, 3, 1, 1, 1, 1, 600, 1.5 FROM c;
         COMMIT;"
    ))
    .unwrap();
}

/// Everything a wipe report counts, added up.
fn touched(report: &SignalWipeReport) -> usize {
    report.events_deleted
        + report.counter_rows_deleted
        + report.plays_deleted
        + report.plays_cleared
        + report.rollups_deleted
        + report.rollups_cleared
        + report.snapshots_deleted
        + report.sessions_deleted
        + report.dictionary_rows_deleted
}

type WipeCall = Box<dyn Fn(&mut SignalWiper) -> usize>;

/// The statements a wipe of one signal category runs, counted off the SQL of `signal_wipe.rs` and
/// `wipe.rs`. Around the work there are always 4: `BEGIN IMMEDIATE`, `COMMIT` and the two
/// checkpoint pragmas. Then 2 deletes (events, counters) and
/// - the category that owns `PlayStarted` (queue and library) deletes the play and rollup rows:
///   2 more, 8 in all;
/// - the others clear the rollups (the score is derived from every kind): a `DELETE` of the rows
///   left empty and an `UPDATE`, and a category with a column or a flag of `play` derived from it
///   (volume, speed, seek/skip/replay, browsing) an `UPDATE play`: 9 in all, 8 without.
fn category_statements(category: SignalCategory) -> usize {
    match category {
        SignalCategory::QueueLibrary | SignalCategory::OutputSound | SignalCategory::Session => 8,
        SignalCategory::Volume
        | SignalCategory::Speed
        | SignalCategory::SeekSkipReplay
        | SignalCategory::Browsing => 9,
    }
}

/// The statements a wipe of the signals of a tier runs (see [`category_statements`]): T0 holds
/// `PlayStarted` (8), T1 has no play column but the score (8), and T2 and T3 have no signal kind:
/// only the 4 around the work, because nothing is scanned.
fn tier_statements(tier: Tier) -> usize {
    match tier {
        Tier::T0 | Tier::T1 => 8,
        Tier::T2 | Tier::T3 => 4,
    }
}

/// A tier wipe with its context fields: the statements of the signal part ([`tier_statements`]) and,
/// when the tier has context fields (T1 to T3; T0 has none), a scan of the stored keys and an
/// `UPDATE` for each of `play` and `context_snapshot`: 4 more.
fn tier_with_context_statements(tier: Tier) -> usize {
    match tier {
        Tier::T0 => 8,
        Tier::T1 => 12,
        Tier::T2 | Tier::T3 => 8,
    }
}

/// Wipe everything: `BEGIN IMMEDIATE`, the read and the write of the wipe epoch, one `DELETE` for
/// each of the ten tables, `COMMIT` and the two checkpoint pragmas.
const EVERYTHING_STATEMENTS: usize = 16;

/// The cost of a wipe is a number of statements, each one scan, **not** a number of rows
/// (`signal_wipe.rs`, "Cost and locking"): the wiper holds the write lock of `signals.db` for its
/// whole transaction, and a statement per row, per distinct key or per kind would hold it for
/// seconds on the reference listener's tables (that is what made the first context wipe starve the
/// writer). The contention tests only show that a batch gets in once the wipe is over, so this
/// pins the cost itself: every kind of wipe runs the same statements on a database of three
/// plays and on one of three thousand, and the number of them is the one counted above.
#[test]
fn a_wipe_runs_a_fixed_number_of_statements_whatever_the_row_count() {
    let mut cases: Vec<(String, usize, WipeCall)> = Vec::new();
    for category in SignalCategory::ALL {
        cases.push((
            format!("category {category:?}"),
            category_statements(category),
            Box::new(move |wiper| touched(&wiper.wipe_signal_categories(&[category]).unwrap())),
        ));
    }
    for tier in Tier::ALL {
        cases.push((
            format!("signal tier {tier}"),
            tier_statements(tier),
            Box::new(move |wiper| touched(&wiper.wipe_signal_tiers(&[tier]).unwrap())),
        ));
    }
    for tier in Tier::ALL {
        cases.push((
            format!("tier {tier} with its context"),
            tier_with_context_statements(tier),
            Box::new(move |wiper| {
                let (signals, context) = wiper.wipe_tiers(&[tier]).unwrap();
                touched(&signals) + context.plays_rewritten + context.snapshots_rewritten
            }),
        ));
    }
    cases.push((
        "everything".to_owned(),
        EVERYTHING_STATEMENTS,
        Box::new(|wiper| touched(&wiper.wipe_everything().unwrap())),
    ));

    let mut wrong = Vec::new();
    for (name, expected, wipe) in cases {
        let mut small = Fixture::new();
        let (small_touched, small_sql) = statements_of(&mut small.wiper, |w| wipe(w));
        let mut large = Fixture::new();
        populate_many(&large.db.reader());
        let (large_touched, large_sql) = statements_of(&mut large.wiper, |w| wipe(w));

        if large_sql != small_sql {
            wrong.push(format!(
                "{name}: the statements depend on the rows: {} on the small database, {} on the large one",
                small_sql.len(),
                large_sql.len()
            ));
        }
        if large_sql.len() != expected {
            wrong.push(format!(
                "{name}: {} statements, expected {expected}: {large_sql:#?}",
                large_sql.len()
            ));
        }
        // The large database really was bigger for the wipe (the tiers and categories that change
        // nothing in `signals.db` change nothing in either).
        if (large_touched == 0 && small_touched != 0)
            || (large_touched > 0 && large_touched <= small_touched)
        {
            wrong.push(format!(
                "{name}: touched {small_touched} rows of the small database and {large_touched} of the large one"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// ---------------------------------------------------------------- contention

/// How long a test waits for a condition of the machine (events stored, a wipe returned) before it
/// calls the wait a failure: far above anything seen with 32 heavy processes on 16 threads, so it
/// only fires on a hang.
const LOAD_BUDGET: Duration = Duration::from_secs(120);

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

/// A lock that outlasts the 5 s busy timeout, as a wipe's does under machine load: a bare
/// `write_batch` gives up with "database is locked", and the retry of the helper (what the writer
/// thread does with a failed commit) stores the batch once the lock is gone. This is what the
/// contention tests below stand on; without it they depended on the wipe finishing in under 5 s.
#[test]
fn a_batch_that_outwaits_the_busy_timeout_is_stored_by_the_retry() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(75);
    let events = play_batch(&mut ids);
    let holder = db.reader();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let wipe_end = common::WipeEnd::new();

    let wipe_end_ref = &wipe_end;
    let (stored, contended) = std::thread::scope(|scope| {
        scope.spawn(move || {
            std::thread::sleep(BUSY_TIMEOUT + Duration::from_secs(1));
            holder.execute_batch("COMMIT").unwrap();
            wipe_end_ref.set(Instant::now()).unwrap();
        });
        write_batch_when_unlocked(&mut store, &events[..1], &wipe_end)
    });

    let batch = stored.unwrap_or_else(|e| panic!("{contended:?}: {}", e.chain()));
    assert_eq!(batch.events_inserted, 1);
    assert!(
        contended.attempts >= 2,
        "the first attempt must have run into the busy timeout: {contended:?}"
    );
    assert!(contended.waited >= BUSY_TIMEOUT, "{contended:?}");
    assert!(contended.after_wipe <= AFTER_WIPE_SLACK, "{contended:?}");
    assert_eq!(count(&db.reader(), "SELECT count(*) FROM signal_event"), 1);
}

/// What a contention case wipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wipe {
    /// A signal category that rewrites the 131 400 play rows.
    Category,
    /// Tiers T1 and T2 in one call: their signals and the context fields of the 131 400 plays.
    Tiers,
    /// Everything.
    Everything,
}

/// A batch the writer sends while the reference-size tables are wiped is stored: for a category
/// wipe (which rewrites the 131 400 play rows), a tier wipe (signals and context in one
/// transaction) and wipe everything. The probe must have seen the wipe's lock, so the contention
/// really happened.
///
/// The writer's batch is offered again when it finds the database locked
/// ([`write_batch_when_unlocked`], as the writer thread does), because under machine load the
/// wipe's lock can outlast the 5 s busy timeout (32 heavy processes made this test fail with
/// "database is locked" after 5.5 s when it relied on the timeout alone). What the test pins
/// instead is that nothing holds the writer up *after* the wipe: the batch is stored within
/// [`AFTER_WIPE_SLACK`] of the wipe's return, which is measured, not assumed.
#[test]
fn a_write_batch_during_a_wipe_of_the_reference_tables_is_stored() {
    for mode in [Wipe::Category, Wipe::Tiers, Wipe::Everything] {
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
            if mode == Wipe::Everything {
                "DELETE ON dict_device"
            } else {
                "UPDATE ON play"
            },
        );
        let rest = events[1..].to_vec();

        let ((signals, context, wipe_time), (batch, contended), lock_seen) = during_a_wipe(
            &db,
            || {
                let started = Instant::now();
                let (signals, context) = match mode {
                    Wipe::Category => (
                        wiper
                            .wipe_signal_categories(&[SignalCategory::SeekSkipReplay])
                            .unwrap(),
                        WipeReport::default(),
                    ),
                    Wipe::Tiers => wiper.wipe_tiers(&[Tier::T1, Tier::T2]).unwrap(),
                    Wipe::Everything => (wiper.wipe_everything().unwrap(), WipeReport::default()),
                };
                (signals, context, started.elapsed())
            },
            |wipe_end| write_batch_when_unlocked(&mut store, &rest, wipe_end),
        );
        eprintln!("{mode:?}: wipe {wipe_time:?}, batch {contended:?}");
        assert!(lock_seen, "{mode:?}: the probe never saw the wipe's lock");
        let batch = batch.unwrap_or_else(|e| {
            panic!(
                "{mode:?}: the batch was not stored: {contended:?} (the wipe took {wipe_time:?}): {}",
                e.chain()
            )
        });
        assert!(
            contended.after_wipe <= AFTER_WIPE_SLACK,
            "{mode:?}: the batch was stored {:?} after the wipe returned",
            contended.after_wipe
        );
        // The writer never waits much longer than the wipe itself: it started waiting after the
        // wipe started, and gets in at the latest slightly after it returned.
        assert!(
            contended.waited <= wipe_time + AFTER_WIPE_SLACK,
            "{mode:?}: the batch waited {:?} for a wipe of {wipe_time:?}",
            contended.waited
        );
        assert_eq!(batch.events_inserted, 1, "{mode:?}");

        let conn = db.reader();
        match mode {
            Wipe::Everything => {
                assert_eq!(signals.plays_deleted, 131_400);
                // The PlayStarted was wiped: the skip has no start to pair with.
                assert_eq!(batch.plays_unpaired, 1);
                assert_eq!(count(&conn, "SELECT count(*) FROM play"), 0);
                assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 1);
                // The writer's caches were stale after the wipe: the row it wrote must still point
                // at dictionary, session and device rows that exist.
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
            }
            Wipe::Category => {
                assert_eq!(signals.plays_cleared, 131_400);
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
            Wipe::Tiers => {
                // T1 and T2 have no signal kind that feeds a play column; the score is reset in the
                // 20 000 rollups; the keys of the plays lose their clock, device, app and place
                // fields in the same transaction.
                assert_eq!(signals.events_deleted, 0);
                assert_eq!(signals.rollups_cleared, 20_000);
                let fields = ContextCategory::tier_fields(Tier::T1)
                    .union(ContextCategory::tier_fields(Tier::T2));
                let changed = (1..=131_400u32)
                    .filter(|x| ContextKey::clear_raw(x % 4000, fields) != x % 4000)
                    .count();
                assert!(
                    changed > 100_000,
                    "the fixture rewrites most keys ({changed})"
                );
                assert_eq!(context.plays_rewritten, changed);
                assert_eq!(batch.plays_written, 1);
                assert_eq!(count(&conn, "SELECT count(*) FROM play"), 131_401);
                let keys: Vec<u32> = ints(
                    &conn,
                    "SELECT DISTINCT context_key FROM play WHERE id <= 131400 ORDER BY context_key",
                );
                let mut expected: Vec<u32> = (0..4000u32)
                    .map(|k| ContextKey::clear_raw(k, fields))
                    .collect();
                expected.sort_unstable();
                expected.dedup();
                assert_eq!(keys, expected);
            }
        }

        // The wiper keeps no lock after it returned. The batch above cannot show one: it got in
        // the moment the wipe committed. A batch sent now would wait out the busy timeout and
        // fail if the wiper still held the write lock (an open transaction on its connection).
        let after = write_batch_after_the_wipe(&mut store, &play_batch(&mut Ids::new(74)));
        assert_eq!(after.events_inserted, 2, "{mode:?}");
        assert_eq!(after.plays_written, 1, "{mode:?}");
    }
}

// ---------------------------------------------------------------- deleted means gone

const MARKER: &[u8] = b"TINWAVE-WIPE-MARKER-0123456789abcdef-xyz";

fn wal_path(db: &TempDb) -> std::path::PathBuf {
    let mut p = db.path.as_os_str().to_owned();
    p.push("-wal");
    std::path::PathBuf::from(p)
}

/// How many times the marker is in the file at `path` (0 when the file does not exist).
fn marker_copies(path: &std::path::Path) -> usize {
    std::fs::read(path).map_or(0, |bytes| {
        bytes.windows(MARKER.len()).filter(|w| *w == MARKER).count()
    })
}

fn checkpoint(conn: &Connection) {
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy, 0, "the checkpoint was blocked");
}

/// Deleted content is zeroed in the main database file, not left in the free pages, and it is gone
/// when the wipe returns, with no checkpoint of the test's own: a wipe that only unlinked the rows
/// would leave the data readable, and one that left the zeroed pages in the WAL would leave the
/// old bytes in the file.
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
        assert_eq!(
            marker_copies(&db.path),
            2,
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
        // No checkpoint here: the wipe folds its zeroed pages into the main file itself, so the
        // content is gone from the file the moment the wipe returns (T-0065).
        let main_copies = marker_copies(&db.path);
        let wal_copies = marker_copies(&wal_path(&db));
        if everything {
            assert_eq!(
                (main_copies, wal_copies),
                (0, 0),
                "wiped data is still readable in the file"
            );
        } else {
            // The snapshot is not part of a category wipe: the event payload copy is zeroed, the
            // snapshot blob's copy stays.
            assert_eq!(
                (main_copies, wal_copies),
                (1, 0),
                "the event payload copy is zeroed, the snapshot's stays"
            );
        }
    }
}

/// The checkpoint after a wipe is best effort: a reader that still sees the pages from before the
/// wipe blocks it, and the wipe must neither wait for it nor fail. The deleted content then stays
/// in the main file until the next checkpoint (the caveat the privacy doc names), and a later
/// checkpoint, once the reader is gone, removes it.
#[test]
fn a_reader_that_blocks_the_checkpoint_does_not_fail_or_delay_the_wipe() {
    let db = TempDb::new();
    SignalStore::open(&db.path).unwrap();
    let conn = db.reader();
    conn.execute_batch(
        "INSERT INTO dict_device (id, uuid) VALUES (1, x'01');
         INSERT INTO session (id, uuid, device, started_ts, tz_offset_min) VALUES (1, x'0f', 1, 1000, 0);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO signal_event (device, seq, session, ts_off, kind, payload) \
         VALUES (1, 1, 1, 0, ?1, ?2)",
        params![SignalKind::VolumeChanged.discriminant(), MARKER],
    )
    .unwrap();
    checkpoint(&conn);
    assert_eq!(marker_copies(&db.path), 1, "the fixture wrote the marker");

    // A read transaction opened before the wipe keeps its snapshot of the old pages.
    let reader = db.reader();
    reader.execute_batch("BEGIN").unwrap();
    assert_eq!(count(&reader, "SELECT count(*) FROM signal_event"), 1);

    let mut wiper = SignalWiper::open(&db.path).unwrap();
    let started = std::time::Instant::now();
    let report = wiper.wipe_everything().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the wipe waited for the reader ({:?})",
        started.elapsed()
    );
    assert_eq!(report.events_deleted, 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM signal_event"), 0);
    assert_eq!(
        marker_copies(&db.path),
        1,
        "the checkpoint was blocked, so the file still holds the old bytes"
    );

    reader.execute_batch("COMMIT").unwrap();
    checkpoint(&conn);
    assert_eq!(marker_copies(&db.path), 0);
}
