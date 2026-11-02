//! The Signal Center toggles survive a restart: `PersistentSettings` against a real `signals.db`
//! in a temp dir, with the real bus checking them before it constructs an event.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::cell::Cell;
use std::error::Error;

use common::{count, favorited, Ids, TempDb, T0};
use rusqlite::Connection;
use tinwave_core::{ManualClock, Tier, UtcMillis};
use tinwave_db::{
    migrate, PersistentSettings, SettingsLoad, SettingsProblem, SignalStore, SignalWriter,
    WriterConfig, SIGNALS_MIGRATIONS,
};
use tinwave_signals::{
    BusConfig, EmitOutcome, SignalBus, SignalCategory, SignalKind, SignalSettings,
};

fn clock() -> ManualClock {
    ManualClock::new(UtcMillis::new(T0))
}

fn open(db: &TempDb) -> PersistentSettings {
    PersistentSettings::open(&db.path, clock()).unwrap()
}

/// A bus on the given settings, as the app builds it after loading them.
fn bus_on(settings: &PersistentSettings, ids: &mut Ids) -> SignalBus {
    let (bus, _drain) = SignalBus::channel(
        BusConfig::new(ids.device()),
        ids.clock.clone(),
        settings.settings().clone(),
    )
    .unwrap();
    bus
}

/// Offers every event kind of `category` to the bus with a counting constructor and returns how
/// many times a constructor ran, and how many events were queued.
fn offer_category(bus: &SignalBus, category: SignalCategory) -> (u32, u32) {
    let built = Cell::new(0_u32);
    let mut queued = 0;
    let mut ids = Ids::new(1);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let event = favorited(device, session, rec, 0);
    for kind in SignalKind::events().filter(|&k| SignalCategory::of(k) == category) {
        let outcome = bus.emit_with(kind, |_| {
            built.set(built.get() + 1);
            // Only the count matters: for any kind but `Favorited` the bus then refuses the
            // event as a payload of another kind.
            event
        });
        match outcome {
            Ok(EmitOutcome::Queued) => queued += 1,
            Ok(EmitOutcome::Disabled) | Ok(EmitOutcome::Dropped) | Err(_) => {}
        }
    }
    (built.get(), queued)
}

fn stored_mask(db: &TempDb) -> Option<i64> {
    db.reader()
        .query_row("SELECT mask FROM signal_settings WHERE id = 1", [], |r| {
            r.get(0)
        })
        .ok()
}

#[test]
fn the_first_run_uses_the_documented_defaults_and_stores_them() {
    let db = TempDb::new();
    let settings = open(&db);
    assert_eq!(settings.load(), &SettingsLoad::FirstRun);
    assert!(
        settings.load_problem().is_none(),
        "a first run is not an error"
    );

    // privacy-and-signals.md: every category on, T0 and T1 on, T2 and T3 (opt-in) off.
    let s = settings.settings();
    for category in SignalCategory::ALL {
        assert!(s.category_enabled(category), "{category:?}");
    }
    assert!(s.tier_enabled(Tier::T0) && s.tier_enabled(Tier::T1));
    assert!(!s.tier_enabled(Tier::T2) && !s.tier_enabled(Tier::T3));
    assert_eq!(s.mask(), SignalSettings::new().mask());
    for &kind in SignalKind::ALL {
        assert!(s.enabled(kind), "{kind} is T0/T1");
    }

    // The row exists from the start, stamped with the injected clock.
    let r = db.reader();
    assert_eq!(count(&r, "SELECT count(*) FROM signal_settings"), 1);
    assert_eq!(count(&r, "SELECT mask FROM signal_settings"), 0x37F);
    assert_eq!(count(&r, "SELECT updated_ts FROM signal_settings"), T0);

    // Opening again is a restore, not another first run, and does not touch the row.
    drop(settings);
    let again =
        PersistentSettings::open(&db.path, ManualClock::new(UtcMillis::new(T0 + 99_000))).unwrap();
    assert_eq!(again.load(), &SettingsLoad::Restored);
    assert_eq!(count(&r, "SELECT updated_ts FROM signal_settings"), T0);
}

#[test]
fn a_category_turned_off_stays_off_after_a_restart_and_nothing_is_constructed() {
    for category in SignalCategory::ALL {
        let db = TempDb::new();
        let mut ids = Ids::new(3);

        // Run 1: switch the category off. Effective at once, for the bus that is already running.
        {
            let settings = open(&db);
            let bus = bus_on(&settings, &mut ids);
            settings.set_category(category, false).unwrap();
            assert_eq!(
                offer_category(&bus, category),
                (0, 0),
                "{category:?} right away"
            );
        }

        // Run 2: a new process. Load, then build the bus.
        let settings = open(&db);
        assert_eq!(settings.load(), &SettingsLoad::Restored);
        assert!(
            !settings.settings().category_enabled(category),
            "{category:?}"
        );
        let bus = bus_on(&settings, &mut ids);
        assert_eq!(
            offer_category(&bus, category),
            (0, 0),
            "{category:?}: no event constructed after the restart"
        );
        // Every other category is still on: the constructors run for them.
        for other in SignalCategory::ALL.into_iter().filter(|&c| c != category) {
            let (built, _) = offer_category(&bus, other);
            let kinds = SignalKind::events()
                .filter(|&k| SignalCategory::of(k) == other)
                .count();
            assert_eq!(
                usize::try_from(built).unwrap(),
                kinds,
                "{other:?} is still on"
            );
        }
    }
}

#[test]
fn tiers_and_several_toggles_are_stored_together() {
    let db = TempDb::new();
    {
        let settings = open(&db);
        settings.set_tier(Tier::T1, false).unwrap();
        settings.set_tier(Tier::T2, true).unwrap();
        settings
            .set_category(SignalCategory::Browsing, false)
            .unwrap();
        settings.set_category(SignalCategory::Speed, false).unwrap();
        settings.set_category(SignalCategory::Speed, true).unwrap();
    }
    let settings = open(&db);
    let s = settings.settings();
    assert!(s.tier_enabled(Tier::T0) && !s.tier_enabled(Tier::T1));
    assert!(s.tier_enabled(Tier::T2) && !s.tier_enabled(Tier::T3));
    assert!(!s.category_enabled(SignalCategory::Browsing));
    assert!(s.category_enabled(SignalCategory::Speed));
    assert!(!s.enabled(SignalKind::OutputDeviceChanged), "T1 is off");
    assert!(!s.enabled(SignalKind::ViewOpened), "Browsing is off");
    assert!(s.enabled(SignalKind::RateChanged));
    assert_eq!(i64::from(s.mask()), stored_mask(&db).unwrap());
}

#[test]
fn a_toggle_writes_settings_and_never_a_signal() {
    let db = TempDb::new();
    let settings = open(&db);
    for category in SignalCategory::ALL {
        settings.set_category(category, false).unwrap();
        settings.set_category(category, true).unwrap();
    }
    settings.set_tier(Tier::T3, true).unwrap();
    let r = db.reader();
    for table in ["signal_event", "ui_counter", "session", "dict_recording"] {
        assert_eq!(
            count(&r, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
}

#[test]
fn a_missing_row_in_a_database_with_signals_is_reported_not_hidden() {
    let db = TempDb::new();
    // A database that already holds a signal (written by the real store) but lost its settings.
    {
        let mut ids = Ids::new(5);
        let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
        let mut store = SignalStore::open(&db.path).unwrap();
        store
            .write_batch(&[favorited(device, session, rec, 0)], &[])
            .unwrap();
    }
    let settings = open(&db);
    assert_eq!(
        settings.load_problem(),
        Some(&SettingsProblem::MissingRow),
        "the Signal Center must show this"
    );
    assert!(settings
        .load_problem()
        .unwrap()
        .to_string()
        .contains("missing"));
    // The documented defaults are in effect, so collection keeps working as documented...
    assert_eq!(settings.settings().mask(), SignalSettings::new().mask());
    // ...the database is left as found until the user changes something...
    assert_eq!(stored_mask(&db), None);
    // ...and then the row is written and the next start is clean.
    settings
        .set_category(SignalCategory::Volume, false)
        .unwrap();
    drop(settings);
    let next = open(&db);
    assert_eq!(next.load(), &SettingsLoad::Restored);
    assert!(!next.settings().category_enabled(SignalCategory::Volume));
}

#[test]
fn a_corrupt_row_falls_back_to_the_defaults_and_says_so() {
    let bad_values: [(&str, &str); 5] = [
        ("'on'", "text"),
        ("x'0102'", "blob"),
        ("-1", "mask -1"),
        ("4096", "mask 4096"),             // bit 12: not a category or tier
        ("4294967296", "mask 4294967296"), // does not fit 32 bits
    ];
    for (literal, expected) in bad_values {
        let db = TempDb::new();
        drop(open(&db));
        db.reader()
            .execute_batch(&format!("UPDATE signal_settings SET mask = {literal}"))
            .unwrap();
        let settings = open(&db);
        let problem = settings
            .load_problem()
            .unwrap_or_else(|| panic!("{literal}: no problem reported"));
        let SettingsProblem::Corrupt { found } = problem else {
            panic!("{literal}: {problem:?}");
        };
        assert!(found.contains(expected), "{literal}: {found}");
        assert!(problem.to_string().contains("not valid"), "{problem}");
        assert_eq!(
            settings.settings().mask(),
            SignalSettings::new().mask(),
            "{literal}"
        );
        // Still there for the user to see on the next start, until they change a toggle.
        assert!(open(&db).load_problem().is_some(), "{literal}");
    }
}

#[test]
fn a_failed_save_is_an_error_but_the_toggle_is_in_effect() {
    let db = TempDb::new();
    let settings = open(&db);
    db.reader()
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE UPDATE ON signal_settings \
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    let error = settings
        .set_category(SignalCategory::Volume, false)
        .expect_err("the row could not be written");
    assert!(
        error.chain().contains("injected failure"),
        "{}",
        error.chain()
    );
    assert!(error.source().is_some(), "SQLite's error is kept");
    assert!(
        !settings.settings().enabled(SignalKind::VolumeChanged),
        "the user's choice is honoured now even though it was not stored"
    );
    assert_eq!(stored_mask(&db), Some(0x37F), "the old row is untouched");
}

#[test]
fn settings_can_be_saved_while_the_writer_is_running() {
    let db = TempDb::new();
    let mut ids = Ids::new(8);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let settings = open(&db);
    let store = SignalStore::open(&db.path).unwrap();
    let (bus, drain) = SignalBus::channel(
        BusConfig::new(device),
        ids.clock.clone(),
        settings.settings().clone(),
    )
    .unwrap();
    let writer = SignalWriter::spawn(store, drain, WriterConfig::default()).unwrap();
    for i in 0..300 {
        let _ = bus
            .emit_with(SignalKind::Favorited, |s| {
                favorited(device, session, rec, s.id.seq)
            })
            .unwrap();
        if i % 50 == 0 {
            settings.set_tier(Tier::T2, i % 100 == 0).unwrap();
        }
    }
    settings
        .set_category(SignalCategory::QueueLibrary, false)
        .unwrap();
    // Favorited is a QueueLibrary kind: now off at the source, the bus builds nothing more.
    let refused = bus
        .emit_with(SignalKind::Favorited, |_| {
            unreachable!("the category is off")
        })
        .unwrap();
    assert_eq!(refused, EmitOutcome::Disabled);
    let stats = writer.shutdown().unwrap();
    assert_eq!(stats.events_inserted, 300);
    assert_eq!(
        count(&db.reader(), "SELECT count(*) FROM signal_event"),
        300
    );
    assert!(stored_mask(&db).is_some());
}

#[test]
fn a_database_from_before_the_settings_table_is_upgraded_without_a_false_alarm() {
    let db = TempDb::new();
    // A build that only knew migration 0001, with a signal already stored.
    {
        let mut conn = Connection::open(&db.path).unwrap();
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")
            .unwrap();
        migrate(&mut conn, &SIGNALS_MIGRATIONS[..1]).unwrap();
    }
    assert_eq!(
        count(
            &db.reader(),
            "SELECT count(*) FROM sqlite_master WHERE name = 'signal_settings'"
        ),
        0
    );
    db.reader()
        .execute_batch(
            "INSERT INTO dict_device (id, uuid) VALUES (1, x'01'); \
             INSERT INTO session (id, uuid, device, started_ts, tz_offset_min) VALUES (1, x'02', 1, 0, 0); \
             INSERT INTO signal_event (device, seq, session, ts_off, kind) VALUES (1, 0, 1, 0, 54);",
        )
        .unwrap();

    // The settings table is new in this open: the defaults are stored, nothing is wrong.
    let settings = open(&db);
    assert_eq!(settings.load(), &SettingsLoad::FirstRun);
    assert!(settings.load_problem().is_none());
    assert_eq!(
        count(&db.reader(), "SELECT count(*) FROM schema_migration"),
        2
    );
    assert_eq!(count(&db.reader(), "SELECT count(*) FROM signal_event"), 1);
    assert_eq!(stored_mask(&db), Some(0x37F));
}
