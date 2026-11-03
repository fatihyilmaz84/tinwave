//! Wiping a context category (T-0060): the fields it owns are cleared from `play.context_key`
//! and `context_snapshot.key` in one transaction. Every test drives the real `SignalWiper` on a
//! SQLite file; failures are injected from a second connection with a trigger, as in
//! `play_records.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::{Duration, Instant};

use common::{count, Ids, TempDb, T0};
use rusqlite::{params, Connection};
use tinwave_core::{ManualClock, SignalEventId, Tier, UtcMillis};
use tinwave_db::{
    ContextCategory, DbError, PersistentSettings, SignalStore, SignalWiper, WipeReport,
};
use tinwave_signals::{
    ActivityGuess, ContextFieldSet, ContextKey, ContextSnapshotId, Daypart, Fraction, InputSource,
    OutputClass, PlaceSlot, PlayOrigin, PlayStartedPayload, QueueOrigin, Section, SignalCategory,
    SignalEvent, SignalPayload, SkipPayload, TemperatureBand, WeatherCondition, WeekdayType,
};

/// The seven fields of a `ContextKey`, by test-local name.
const ALL_FIELDS: [FieldName; 7] = [
    FieldName::Daypart,
    FieldName::Weekday,
    FieldName::Condition,
    FieldName::Temperature,
    FieldName::Device,
    FieldName::Activity,
    FieldName::Place,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldName {
    Daypart,
    Weekday,
    Condition,
    Temperature,
    Device,
    Activity,
    Place,
}

/// A key with exactly the listed fields known, built only with the typed builders (not with the
/// code under test), so the expectations do not share an implementation with the wipe.
fn key_of(fields: &[FieldName]) -> ContextKey {
    let mut key = ContextKey::UNKNOWN;
    for field in fields {
        key = match field {
            FieldName::Daypart => key.with_daypart(Daypart::Evening),
            FieldName::Weekday => key.with_weekday_type(WeekdayType::Weekend),
            FieldName::Condition => key.with_condition(WeatherCondition::Rain),
            FieldName::Temperature => key.with_temperature(TemperatureBand::Mild),
            FieldName::Device => key.with_device_class(OutputClass::Bluetooth),
            FieldName::Activity => key.with_activity(ActivityGuess::Leisure),
            FieldName::Place => key.with_place(PlaceSlot::new(5).unwrap()),
        };
    }
    key
}

fn full() -> ContextKey {
    key_of(&ALL_FIELDS)
}

/// All fields but those in `without`.
fn full_without(without: &[FieldName]) -> ContextKey {
    let kept: Vec<FieldName> = ALL_FIELDS
        .iter()
        .copied()
        .filter(|f| !without.contains(f))
        .collect();
    key_of(&kept)
}

/// The fields a wipe of each category clears (the ones it owns plus those derived from it),
/// written out here (see the table in `wipe.rs`).
fn owned(category: ContextCategory) -> &'static [FieldName] {
    match category {
        ContextCategory::Clock => &[FieldName::Daypart, FieldName::Weekday, FieldName::Activity],
        ContextCategory::OutputDevice => &[FieldName::Device],
        ContextCategory::ForegroundApp => &[FieldName::Activity],
        ContextCategory::NetworkPlace => &[FieldName::Place, FieldName::Activity],
        ContextCategory::Weather => &[FieldName::Condition, FieldName::Temperature],
    }
}

/// A `signals.db` with `play` rows and `context_snapshot` rows carrying the given keys.
struct Fixture {
    db: TempDb,
    wiper: SignalWiper,
}

impl Fixture {
    fn new(play_keys: &[u32], snapshot_keys: &[u32]) -> Self {
        let db = TempDb::new();
        SignalStore::open(&db.path).unwrap();
        let conn = db.reader();
        // One transaction: the reference-size tests insert a hundred thousand rows.
        conn.execute_batch("BEGIN;").unwrap();
        {
            let mut insert = conn
                .prepare_cached(
                    "INSERT INTO play (device, session, recording, started_ts, listened_ds, \
                     end_reason, end_frac, flags, context_key) \
                     VALUES (1, 1, ?1, ?2, 600, 0, 255, 5, ?3)",
                )
                .unwrap();
            for (i, key) in play_keys.iter().enumerate() {
                let i = i64::try_from(i).unwrap();
                insert
                    .execute(params![100 + i, T0 + i * 1_000, key])
                    .unwrap();
            }
            let mut insert = conn
                .prepare_cached(
                    "INSERT INTO context_snapshot (id, hash, key, fields) \
                     VALUES (?1, ?2, ?3, x'00')",
                )
                .unwrap();
            for (i, key) in snapshot_keys.iter().enumerate() {
                let i = i64::try_from(i).unwrap();
                insert.execute(params![i + 1, (i + 1) * 7919, key]).unwrap();
            }
        }
        conn.execute_batch("COMMIT;").unwrap();
        let wiper = SignalWiper::open(&db.path).unwrap();
        Self { db, wiper }
    }

    fn play_keys(&self) -> Vec<u32> {
        column(
            &self.db.reader(),
            "SELECT context_key FROM play ORDER BY id",
        )
    }

    fn snapshot_keys(&self) -> Vec<u32> {
        column(
            &self.db.reader(),
            "SELECT key FROM context_snapshot ORDER BY id",
        )
    }

    /// Every column of `play` except the key: a wipe must not touch them.
    fn play_rest(&self) -> Vec<Vec<i64>> {
        let conn = self.db.reader();
        let mut stmt = conn
            .prepare(
                "SELECT id, device, session, recording, started_ts, listened_ds, end_reason, \
                 end_frac, flags FROM play ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |r| (0..9).map(|i| r.get(i)).collect())
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

fn column(conn: &Connection, sql: &str) -> Vec<u32> {
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

// ---------------------------------------------------------------- per category

/// Wiping each category zeroes exactly the fields it owns, in every play row and every snapshot:
/// the other fields keep their values, and the other columns of the row are untouched.
#[test]
fn wiping_a_category_clears_exactly_its_fields_in_every_play_row() {
    for category in ContextCategory::ALL {
        let own = owned(category);
        // One key of every field; one with a single other field; one with only the owned fields;
        // a never-captured 0.
        let others: Vec<FieldName> = ALL_FIELDS
            .iter()
            .copied()
            .filter(|f| !own.contains(f))
            .collect();
        let before = [
            full().raw(),
            key_of(&[others[0]]).raw(),
            key_of(own).raw(),
            0,
            full().raw(),
        ];
        let mut fx = Fixture::new(&before, &before);
        let rest = fx.play_rest();

        let report = fx.wiper.wipe_context(&[category]).unwrap();

        let after = [
            full_without(own).raw(),
            key_of(&[others[0]]).raw(),
            0, // its last known fields went: the all-unknown raw 0
            0,
            full_without(own).raw(),
        ];
        assert_eq!(fx.play_keys(), after, "{category:?}: play rows");
        assert_eq!(fx.snapshot_keys(), after, "{category:?}: snapshots");
        assert_eq!(
            report,
            WipeReport {
                plays_rewritten: 3,
                snapshots_rewritten: 3
            },
            "{category:?}"
        );
        assert_eq!(fx.play_rest(), rest, "{category:?}: other columns changed");
        // What stayed is still a key the readers accept.
        for raw in fx.play_keys() {
            ContextKey::from_raw(raw).unwrap();
        }
    }
}

/// A key whose last known field is wiped becomes the raw 0, not a version-1 key with empty fields.
#[test]
fn a_key_whose_last_field_is_wiped_becomes_raw_zero() {
    let only_place = key_of(&[FieldName::Place]).raw();
    let only_weather = key_of(&[FieldName::Condition, FieldName::Temperature]).raw();
    assert_eq!(only_place >> 28, 1, "a known field carries the version");
    let mut fx = Fixture::new(&[only_place, only_weather], &[only_place]);

    fx.wiper
        .wipe_context(&[ContextCategory::NetworkPlace])
        .unwrap();
    assert_eq!(fx.play_keys(), [0, only_weather]);
    assert_eq!(fx.snapshot_keys(), [0]);

    fx.wiper.wipe_context(&[ContextCategory::Weather]).unwrap();
    assert_eq!(fx.play_keys(), [0, 0]);
}

/// Each tier wipes the fields of its categories; T0 owns no context.
#[test]
fn wiping_a_tier_clears_the_fields_of_its_categories() {
    let cases: [(Tier, &[FieldName]); 4] = [
        (Tier::T0, &[]),
        (
            Tier::T1,
            &[
                FieldName::Daypart,
                FieldName::Weekday,
                FieldName::Device,
                FieldName::Activity,
            ],
        ),
        (Tier::T2, &[FieldName::Activity, FieldName::Place]),
        (Tier::T3, &[FieldName::Condition, FieldName::Temperature]),
    ];
    for (tier, cleared) in cases {
        let mut fx = Fixture::new(&[full().raw()], &[full().raw()]);
        let report = fx.wiper.wipe_context_tier(tier).unwrap();
        assert_eq!(fx.play_keys(), [full_without(cleared).raw()], "{tier}");
        assert_eq!(fx.snapshot_keys(), [full_without(cleared).raw()], "{tier}");
        let rewritten = usize::from(!cleared.is_empty());
        assert_eq!(
            (report.plays_rewritten, report.snapshots_rewritten),
            (rewritten, rewritten),
            "{tier}"
        );
    }
}

/// The activity guess is derived from the place and the clock, so wiping the network place or the
/// clock clears it as well as the foreground app that owns it; the device class is not derived from
/// any of them and stays.
#[test]
fn wiping_the_place_or_the_clock_also_clears_the_derived_activity() {
    let cases: [(ContextCategory, &[FieldName]); 3] = [
        (ContextCategory::NetworkPlace, &[FieldName::Place]),
        (
            ContextCategory::Clock,
            &[FieldName::Daypart, FieldName::Weekday],
        ),
        (ContextCategory::ForegroundApp, &[]),
    ];
    for (category, owned_fields) in cases {
        let mut fx = Fixture::new(&[full().raw()], &[full().raw()]);
        fx.wiper.wipe_context(&[category]).unwrap();
        let mut cleared = owned_fields.to_vec();
        cleared.push(FieldName::Activity);
        let key = ContextKey::from_raw(fx.play_keys()[0]).unwrap();
        assert_eq!(key, full_without(&cleared), "{category:?}");
        assert_eq!(key.activity(), None, "{category:?}");
        assert_eq!(
            key.device_class(),
            Some(OutputClass::Bluetooth),
            "{category:?}: the device class is untouched"
        );
        assert_eq!(fx.snapshot_keys(), fx.play_keys(), "{category:?}");
    }
    // The output device and weather wipes never touch activity.
    for category in [ContextCategory::OutputDevice, ContextCategory::Weather] {
        let mut fx = Fixture::new(&[full().raw()], &[]);
        fx.wiper.wipe_context(&[category]).unwrap();
        let key = ContextKey::from_raw(fx.play_keys()[0]).unwrap();
        assert_eq!(key.activity(), Some(ActivityGuess::Leisure), "{category:?}");
    }
}

/// Several categories in one call, and a repeated wipe, which finds nothing more to change.
#[test]
fn several_categories_at_once_and_a_second_wipe_changes_nothing() {
    let mut fx = Fixture::new(&[full().raw()], &[]);
    let both = [ContextCategory::NetworkPlace, ContextCategory::Weather];
    let report = fx.wiper.wipe_context(&both).unwrap();
    assert_eq!(
        fx.play_keys(),
        [full_without(&[
            FieldName::Place,
            FieldName::Activity,
            FieldName::Condition,
            FieldName::Temperature
        ])
        .raw()]
    );
    assert_eq!(report.plays_rewritten, 1);
    assert_eq!(fx.wiper.wipe_context(&both).unwrap(), WipeReport::default());
    assert_eq!(fx.wiper.wipe_context(&[]).unwrap(), WipeReport::default());
}

/// Rows share keys, so one statement rewrites many rows; rows already without the field are not
/// counted.
#[test]
fn many_rows_with_the_same_key_are_all_rewritten() {
    let with = full().raw();
    let without = full_without(&[FieldName::Place, FieldName::Activity]).raw();
    let keys: Vec<u32> = (0..50)
        .map(|i| if i % 5 == 0 { without } else { with })
        .collect();
    let mut fx = Fixture::new(&keys, &[]);
    let report = fx
        .wiper
        .wipe_context(&[ContextCategory::NetworkPlace])
        .unwrap();
    assert_eq!(report.plays_rewritten, 40);
    assert!(fx.play_keys().iter().all(|k| *k == without));
}

// ---------------------------------------------------------------- atomic

/// A failure after the play rows were rewritten leaves play rows and snapshots as they were.
#[test]
fn a_failure_after_the_play_rows_were_rewritten_changes_nothing() {
    let keys = [
        full().raw(),
        key_of(&[FieldName::Place, FieldName::Daypart]).raw(),
    ];
    let mut fx = Fixture::new(&keys, &keys);
    let rest = fx.play_rest();
    // The snapshots are rewritten after the plays: making that fail proves the play rewrite is
    // in the same transaction.
    let injector = fx.db.reader();
    injector
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE UPDATE ON context_snapshot \
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();

    let error = fx
        .wiper
        .wipe_context(&[ContextCategory::NetworkPlace])
        .unwrap_err();
    assert!(
        error.chain().contains("injected failure"),
        "{}",
        error.chain()
    );
    assert_eq!(fx.play_keys(), keys, "play rows are unchanged");
    assert_eq!(fx.snapshot_keys(), keys, "hot-tier snapshots are unchanged");
    assert_eq!(fx.play_rest(), rest);

    // Once the fault is gone the same wipe works.
    injector
        .execute_batch("DROP TRIGGER inject_failure;")
        .unwrap();
    let report = fx
        .wiper
        .wipe_context(&[ContextCategory::NetworkPlace])
        .unwrap();
    assert_eq!((report.plays_rewritten, report.snapshots_rewritten), (2, 2));
    assert_eq!(
        fx.play_keys(),
        [
            full_without(&[FieldName::Place, FieldName::Activity]).raw(),
            key_of(&[FieldName::Daypart]).raw()
        ]
    );
}

/// A failure in the middle of the play rewrite (some distinct keys done, one failing) also rolls
/// back what was already rewritten.
#[test]
fn a_failure_in_the_middle_of_the_play_rewrite_changes_nothing() {
    let (a, b) = (
        full().raw(),
        key_of(&[FieldName::Place, FieldName::Activity]).raw(),
    );
    assert!(
        a > b,
        "keys are rewritten in ascending order: b first, then a"
    );
    let keys = [a, b, a, b];
    let mut fx = Fixture::new(&keys, &keys);
    fx.db
        .reader()
        .execute_batch(&format!(
            "CREATE TRIGGER inject_failure BEFORE UPDATE ON play WHEN OLD.context_key = {a} \
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;"
        ))
        .unwrap();
    let error = fx
        .wiper
        .wipe_context(&[ContextCategory::NetworkPlace])
        .unwrap_err();
    assert!(
        error.chain().contains("injected failure"),
        "{}",
        error.chain()
    );
    // `b` was rewritten to 0 before `a` failed; it is back.
    assert_eq!(fx.play_keys(), keys);
    assert_eq!(fx.snapshot_keys(), keys);
}

/// A stored key that is not a `u32` cannot be cleaned: the wipe fails loudly and rolls back,
/// instead of reporting success with data left behind.
#[test]
fn a_stored_key_that_is_not_a_u32_fails_the_wipe_and_changes_nothing() {
    let mut fx = Fixture::new(&[full().raw()], &[]);
    fx.db
        .reader()
        .execute(
            "INSERT INTO context_snapshot (id, hash, key, fields) VALUES (1, 1, -5, x'00')",
            [],
        )
        .unwrap();
    let error = fx
        .wiper
        .wipe_context(&[ContextCategory::Weather])
        .unwrap_err();
    assert!(matches!(error, DbError::OutOfRange { .. }), "{error}");
    assert_eq!(fx.play_keys(), [full().raw()]);
}

// ---------------------------------------------------------------- what a wipe does not touch

/// Turning a category or tier off stops new values only; the rows change on a wipe and only then.
#[test]
fn turning_a_category_off_leaves_existing_rows_alone() {
    let mut fx = Fixture::new(&[full().raw()], &[full().raw()]);
    let clock = ManualClock::new(UtcMillis::new(T0));
    let settings = PersistentSettings::open(&fx.db.path, clock).unwrap();
    settings.set_tier(Tier::T3, true).unwrap();
    settings.set_tier(Tier::T3, false).unwrap();
    settings.set_tier(Tier::T2, false).unwrap();
    settings
        .set_category(SignalCategory::OutputSound, false)
        .unwrap();
    assert_eq!(fx.play_keys(), [full().raw()]);
    assert_eq!(fx.snapshot_keys(), [full().raw()]);

    fx.wiper.wipe_context_tier(Tier::T3).unwrap();
    assert_eq!(
        fx.play_keys(),
        [full_without(&[FieldName::Condition, FieldName::Temperature]).raw()]
    );
}

/// The whole path: a play whose start points at a snapshot gets that snapshot's key; after the
/// wipe the row has lost the category's fields; events and monthly rollups are untouched.
#[test]
fn a_real_play_loses_the_wiped_fields_and_events_and_rollups_stay() {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(60);
    let (device, session, rec, source) =
        (ids.device(), ids.session(), ids.recording(), ids.source());
    let conn = db.reader();
    conn.execute(
        "INSERT INTO context_snapshot (id, hash, key, fields) VALUES (1, 1, ?1, x'00')",
        [full().raw()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO rollup_monthly (month, recording, plays, completes, skips_early, skips_late, \
         fav_delta, listened_s, score_sum) VALUES (202611, 1, 3, 1, 1, 1, 0, 600, 1.5)",
        [],
    )
    .unwrap();
    let make = |seq: u64, at: i64, ctx: Option<u64>, payload: SignalPayload| SignalEvent {
        id: SignalEventId::new(device, seq),
        ts: UtcMillis::new(T0 + at),
        session,
        recording: Some(rec),
        source: Some(source),
        pos_ds: None,
        payload,
        ctx: ctx.map(ContextSnapshotId),
    };
    let events = [
        make(
            0,
            0,
            Some(1),
            SignalPayload::PlayStarted(PlayStartedPayload {
                origin: PlayOrigin::Queue,
                input: InputSource::Auto,
            }),
        ),
        make(
            1,
            90_000,
            Some(1),
            SignalPayload::Skipped(SkipPayload {
                fraction: Fraction::from_raw(100),
                input: InputSource::Mouse,
                section: Section::UNKNOWN,
                queue_origin: QueueOrigin::Manual,
            }),
        ),
    ];
    let report = store.write_batch(&events, &[]).unwrap();
    assert_eq!(report.plays_written, 1);
    let play_key = |c: &Connection| column(c, "SELECT context_key FROM play");
    assert_eq!(play_key(&conn), [full().raw()]);
    let events_before = count(&conn, "SELECT count(*) FROM signal_event");

    let mut wiper = SignalWiper::open(&db.path).unwrap();
    wiper.wipe_context(&[ContextCategory::Weather]).unwrap();

    let expected = full_without(&[FieldName::Condition, FieldName::Temperature]).raw();
    assert_eq!(play_key(&conn), [expected]);
    assert_eq!(
        column(&conn, "SELECT key FROM context_snapshot"),
        [expected]
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM signal_event"),
        events_before,
        "raw events are not rewritten"
    );
    assert_eq!(
        count(&conn, "SELECT plays + listened_s FROM rollup_monthly"),
        603,
        "the monthly rollup carries no context and is untouched"
    );
}

// ---------------------------------------------------------------- many keys, big tables

/// A small deterministic generator (xorshift), so a failure is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A value in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
    }
}

/// A valid key whose every field is random; a field is unknown one time in `len + 1`.
fn random_key(rng: &mut Rng) -> u32 {
    fn pick<T: Copy>(rng: &mut Rng, all: &[T]) -> Option<T> {
        let i = rng.below(all.len() + 1);
        all.get(i).copied()
    }
    let mut key = ContextKey::UNKNOWN;
    if let Some(v) = pick(rng, Daypart::ALL) {
        key = key.with_daypart(v);
    }
    if let Some(v) = pick(rng, WeekdayType::ALL) {
        key = key.with_weekday_type(v);
    }
    if let Some(v) = pick(rng, WeatherCondition::ALL) {
        key = key.with_condition(v);
    }
    if let Some(v) = pick(rng, TemperatureBand::ALL) {
        key = key.with_temperature(v);
    }
    if let Some(v) = pick(rng, OutputClass::ALL) {
        key = key.with_device_class(v);
    }
    if let Some(v) = pick(rng, ActivityGuess::ALL) {
        key = key.with_activity(v);
    }
    // Slot 0 is "unknown" and cannot be constructed: the place stays unknown then.
    let slot = u8::try_from(rng.below(64)).unwrap();
    if let Ok(slot) = PlaceSlot::new(slot) {
        key = key.with_place(slot);
    }
    key.raw()
}

/// Keys no `ContextKey` produces but a database may hold: the wipe works on bits and must agree
/// with `ContextKey::clear_raw` on them too (a version nibble alone, reserved bits, other
/// versions, undefined field codes).
const ODD_KEYS: [u32; 7] = [
    1 << 28,
    (1 << 28) | (1 << 24),
    3,
    0xDEAD_BEEF,
    0xFFFF_FFFF,
    0x0000_0001,
    (2 << 28) | (7 << 5),
];

/// The semantics are `ContextKey::clear_raw`, row by row: hundreds of distinct random keys (many
/// more rows than keys) plus odd ones, for every category and every tier, in both tables.
#[test]
fn many_distinct_keys_are_rewritten_exactly_as_clear_raw_does() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut distinct: Vec<u32> = (0..600).map(|_| random_key(&mut rng)).collect();
    distinct.extend(ODD_KEYS);
    distinct.push(0);
    distinct.sort_unstable();
    distinct.dedup();
    assert!(distinct.len() > 400, "{} distinct keys", distinct.len());
    // Several rows per key, in a shuffled order, and the snapshots in another one.
    let mut rows = || -> Vec<u32> {
        (0..3_000)
            .map(|_| distinct[rng.below(distinct.len())])
            .collect()
    };
    let plays = rows();
    let snapshots = rows();

    let mut wipes: Vec<(
        String,
        ContextFieldSet,
        Option<ContextCategory>,
        Option<Tier>,
    )> = Vec::new();
    for category in ContextCategory::ALL {
        wipes.push((
            format!("{category:?}"),
            category.fields(),
            Some(category),
            None,
        ));
    }
    for tier in [Tier::T1, Tier::T2, Tier::T3] {
        wipes.push((
            tier.to_string(),
            ContextCategory::tier_fields(tier),
            None,
            Some(tier),
        ));
    }
    for (name, fields, category, tier) in wipes {
        let mut fx = Fixture::new(&plays, &snapshots);
        let report = match (category, tier) {
            (Some(category), _) => fx.wiper.wipe_context(&[category]).unwrap(),
            (None, Some(tier)) => fx.wiper.wipe_context_tier(tier).unwrap(),
            (None, None) => unreachable!(),
        };
        let expect = |keys: &[u32]| -> Vec<u32> {
            keys.iter()
                .map(|k| ContextKey::clear_raw(*k, fields))
                .collect()
        };
        assert_eq!(fx.play_keys(), expect(&plays), "{name}: play rows");
        assert_eq!(fx.snapshot_keys(), expect(&snapshots), "{name}: snapshots");
        let changed = |keys: &[u32]| {
            keys.iter()
                .filter(|k| ContextKey::clear_raw(**k, fields) != **k)
                .count()
        };
        assert_eq!(
            report,
            WipeReport {
                plays_rewritten: changed(&plays),
                snapshots_rewritten: changed(&snapshots),
            },
            "{name}"
        );
    }
}

/// The reference listener's `play` table (120 plays a day for 3 years, 131 400 rows) with
/// thousands of distinct keys. A wipe must hold the write lock for about a scan, not one scan per
/// key: with the one-statement-per-key rewrite it ran for tens of seconds, the writer's batches
/// waited out their 5 s busy timeout and failed with "database is locked".
///
/// The batch is issued while the wipe runs (the probe sees its lock) and must be stored.
#[test]
fn a_write_batch_during_a_wipe_of_the_reference_table_is_not_starved() {
    const PLAYS: usize = 131_400;
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut distinct: Vec<u32> = (0..4_000).map(|_| random_key(&mut rng)).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(distinct.len() >= 1_500, "{} distinct keys", distinct.len());
    let plays: Vec<u32> = (0..PLAYS).map(|i| distinct[i % distinct.len()]).collect();
    let Fixture { db, mut wiper } = Fixture::new(&plays, &distinct);

    let mut store = SignalStore::open(&db.path).unwrap();
    let mut ids = Ids::new(61);
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
    let events = [
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
    ];

    let probe = db.reader();
    probe.busy_timeout(Duration::ZERO).unwrap();
    let (report, wipe_time, batch_time, batch) = std::thread::scope(|scope| {
        let wipe = scope.spawn(|| {
            let started = Instant::now();
            let report = wiper
                .wipe_context(&[ContextCategory::NetworkPlace])
                .unwrap();
            (report, started.elapsed())
        });
        // Wait until the wipe holds the write lock (or has already finished).
        while probe.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").is_ok() && !wipe.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let started = Instant::now();
        let batch = store.write_batch(&events, &[]);
        let batch_time = started.elapsed();
        let (report, wipe_time) = wipe.join().unwrap();
        (report, wipe_time, batch_time, batch)
    });

    let batch = batch.unwrap_or_else(|e| {
        panic!(
            "the batch failed after {batch_time:?} (the wipe took {wipe_time:?}): {}",
            e.chain()
        )
    });
    assert_eq!(batch.plays_written, 1);
    // The wipe did its work on the big table, and the new play is there too.
    let conn = db.reader();
    assert_eq!(
        count(&conn, "SELECT count(*) FROM play"),
        i64::try_from(PLAYS).unwrap() + 1
    );
    assert!(report.plays_rewritten > 0);
    let keys = column(
        &conn,
        "SELECT context_key FROM play ORDER BY id LIMIT 131400",
    );
    let expect: Vec<u32> = plays
        .iter()
        .map(|k| ContextKey::clear_raw(*k, ContextCategory::NetworkPlace.fields()))
        .collect();
    assert_eq!(keys, expect);
}
