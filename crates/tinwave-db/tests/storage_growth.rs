//! Storage growth test v1 (ADR 0002, `docs/rules/testing.md`): a synthetic heavy listener, driven
//! through the real bus, coalescers, counters and `SignalWriter` into `signals.db`, measured against
//! the budgets and the catalog's volume table.
//!
//! Default run (fast): the P-0010 burst assertion and a 3-day determinism run.
//!
//! The 90-day run takes about 15 s (the writer's commits, not the generator, are the cost; a debug
//! build is as fast because SQLite is compiled optimized), so it is in the default run. Its report
//! is printed with `--nocapture`:
//!
//! ```text
//! cargo test -p tinwave-db --release --test storage_growth -- --nocapture
//! ```
//!
//! The expected values are those of `docs/guides/signal-catalog.md` section 9 (see the constants in
//! `growth/mod.rs`, each with the row it comes from); `the_catalog_still_says_what_the_test_expects`
//! fails if the catalog text and those constants drift apart.
//!
//! v1 has no compaction and no play records (E-01, T-0029): it measures the hot tier and
//! extrapolates. `growth::run` calls a hook at the end of every simulated day, at a barrier where
//! the writer is idle; the 5-year run with compaction runs its compaction pass there.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
mod growth;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use common::{Ids, TempDb};
use growth::{measure, run, GrowthConfig};
use rusqlite::Connection;
use tinwave_db::{SignalStore, SignalWriter, WriterConfig};
use tinwave_signals::{
    BusConfig, ContextSnapshotId, ElapsedDs, EventContext, GestureCoalescer, GestureTick,
    GestureTime, InputSource, OutputClass, Section, SignalBus, SignalKind, SignalPayload,
    SignalSettings, SystemVolumePayload, Volume,
};

fn stored(conn: &Connection, kind: SignalKind) -> Vec<Option<Vec<u8>>> {
    let mut stmt = conn
        .prepare("SELECT payload FROM signal_event WHERE kind = ?1 ORDER BY seq")
        .unwrap();
    stmt.query_map([kind.discriminant()], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// P-0010 / ADR 0004: the OS sends one callback per volume step. A 60-step burst within 200 ms
/// must be stored as exactly one `SystemVolumeChanged`, with the first `from` and the last `to`,
/// through the real coalescer, bus, writer and database.
#[test]
fn a_sixty_step_system_volume_burst_within_200_ms_stores_one_event() {
    let db = TempDb::new();
    let mut ids = Ids::new(7);
    let (device, session, rec) = (ids.device(), ids.session(), ids.recording());
    let store = SignalStore::open(&db.path).unwrap();
    let (bus, drain) = SignalBus::channel(
        BusConfig::new(device),
        ids.clock.clone(),
        Arc::new(SignalSettings::new()),
    )
    .unwrap();
    let writer = SignalWriter::spawn(store, drain, WriterConfig::default()).unwrap();
    let mut os_volume = GestureCoalescer::new(bus.clone());
    let context = EventContext {
        session,
        recording: Some(rec),
        source: None,
        pos_ds: Some(1_200),
        ctx: Some(ContextSnapshotId(1)),
    };

    let burst = |coalescer: &mut GestureCoalescer, from: u8, to: u8| {
        // 60 steps, 3 ms apart: 177 ms between the first and the last callback.
        for i in 0..60u8 {
            ids.clock.advance(3);
            let (step_from, step_to) = if to > from {
                (from + i, from + i + 1)
            } else {
                (from - i, from - i - 1)
            };
            let _ = coalescer.system_volume(|| GestureTick {
                context,
                payload: SystemVolumePayload {
                    from: Volume::from_raw(step_from),
                    to: Volume::from_raw(step_to),
                    gesture: GestureTime::from_ms(0),
                    input: InputSource::System,
                    section: Section::UNKNOWN,
                    since_track_start: ElapsedDs::from_ds(120),
                    since_section_start: ElapsedDs::from_ds(120),
                    endpoint_class: OutputClass::Headphones,
                },
            });
        }
    };

    burst(&mut os_volume, 100, 160);
    // Nothing is stored while the burst is open, and one event once 400 ms of silence passed.
    assert_eq!(os_volume.flush_due(), 0, "still inside the 400 ms window");
    ids.clock.advance(400);
    assert_eq!(os_volume.flush_due(), 1);
    // A second burst well after the first one is another gesture, another event.
    ids.clock.advance(5_000);
    burst(&mut os_volume, 160, 100);
    ids.clock.advance(400);
    assert_eq!(os_volume.flush_due(), 1);

    let stats = writer.shutdown().unwrap();
    assert_eq!(stats.events_inserted, 2);
    let conn = Connection::open(&db.path).unwrap();
    let payloads = stored(&conn, SignalKind::SystemVolumeChanged);
    assert_eq!(payloads.len(), 2, "two bursts, two events, not 120");
    let decoded = |i: usize| match SignalPayload::decode(
        SignalKind::SystemVolumeChanged,
        payloads[i].as_deref(),
    )
    .unwrap()
    {
        SignalPayload::SystemVolumeChanged(p) => p,
        other => panic!("decoded {other:?}"),
    };
    let (first, second) = (decoded(0), decoded(1));
    assert_eq!(
        (first.from, first.to),
        (Volume::from_raw(100), Volume::from_raw(160))
    );
    assert_eq!(
        (second.from, second.to),
        (Volume::from_raw(160), Volume::from_raw(100))
    );
    assert!(
        first.gesture.ms() >= 150 && first.gesture.ms() <= 200,
        "gesture {} ms",
        first.gesture.ms()
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM signal_event", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

/// `(seq, session, ts_off, kind, recording, payload)` of a stored event.
type EventRow = (i64, i64, i64, i64, i64, Option<Vec<u8>>);

/// Same seed, same data: two runs produce identical tables, sizes and event counts.
#[test]
fn the_same_seed_gives_the_same_sizes() {
    let cfg = GrowthConfig::reference(3);
    let a = run(cfg, &mut |_| {});
    let b = run(cfg, &mut |_| {});
    let (ra, rb) = (measure(&a), measure(&b));
    assert_eq!(ra.file_bytes, rb.file_bytes);
    assert_eq!(ra.tables, rb.tables);
    assert_eq!(ra.events, rb.events);
    assert_eq!(ra.counter_rows, rb.counter_rows);
    assert_eq!(ra.context_rows, rb.context_rows);
    // Not just the sizes: the stored events themselves are identical.
    let digest = |db: &TempDb| -> Vec<EventRow> {
        let conn = Connection::open(&db.path).unwrap();
        let mut stmt = conn
            .prepare("SELECT seq, session, ts_off, kind, coalesce(recording, 0), payload FROM signal_event")
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
    };
    assert_eq!(digest(&a.db), digest(&b.db));
    // A different seed is different data.
    let other = run(GrowthConfig { seed: 7, ..cfg }, &mut |_| {});
    assert_ne!(measure(&other).file_bytes, ra.file_bytes);

    // And the run was clean: nothing dropped, nothing failed.
    for r in [&a, &b] {
        assert_eq!((r.dropped, r.reserved_overflows, r.keys_dropped), (0, 0, 0));
        assert_eq!(r.writer.failed_commits, 0);
        assert_eq!(r.writer.events_rejected, 0);
    }
}

/// The 90-day run, shared by the tests below (one run, one report).
fn ninety_days() -> &'static (growth::Run, growth::Report) {
    static RUN: OnceLock<(growth::Run, growth::Report)> = OnceLock::new();
    RUN.get_or_init(|| {
        let r = run(GrowthConfig::reference(90), &mut |_| {});
        let report = measure(&r);
        println!("{}", report.render(&r));
        (r, report)
    })
}

/// 90 simulated days of the reference heavy user: nothing may be lost on the way, the hot
/// window must fit 6 MB, the 5-year extrapolation must fit 60 MB, and the run must be fast.
#[test]
fn ninety_days_of_a_heavy_listener_fit_the_hot_window_and_the_five_year_budget() {
    let (r, report) = ninety_days();
    assert_eq!((r.dropped, r.reserved_overflows, r.keys_dropped), (0, 0, 0));
    assert_eq!((r.writer.failed_commits, r.writer.events_lost), (0, 0));
    let violations = report.budget_violations();
    assert!(
        violations.is_empty(),
        "
{}",
        violations.join(
            "
"
        )
    );
    assert!(
        r.elapsed < Duration::from_secs(60),
        "took {:?}, budget 60 s",
        r.elapsed
    );
}

/// The same run against the volume table of `signal-catalog.md` section 9: every event group's
/// and the UI counters' bytes/day within +50 % of the catalog, and the generator within 25 % of
/// the catalog's rates.
#[test]
fn ninety_days_match_the_catalog_volume_table_within_fifty_percent() {
    let (_, report) = ninety_days();
    let violations = report.catalog_violations();
    assert!(
        violations.is_empty(),
        "
{}",
        violations.join(
            "
"
        )
    );
}

/// Context snapshots against the catalog's "~60 a day, ~4.5 KB" (about 70 B per row, the 64-bit
/// hash with its UNIQUE index and the key included): rate within 25%, bytes/day within +50%.
#[test]
fn ninety_days_context_snapshots_match_the_catalog_estimate() {
    let (_, report) = ninety_days();
    let violations = report.context_violations();
    assert!(
        violations.is_empty(),
        "
{}",
        violations.join(
            "
"
        )
    );
}

/// The numbers the harness compares with are copied from the catalog; this ties them to it.
#[test]
fn the_catalog_still_says_what_the_test_expects() {
    let catalog = include_str!("../../../docs/guides/signal-catalog.md").replace(
        "
", "
",
    );
    let row = |prefix: &str| -> String {
        catalog
            .lines()
            .find(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("no row starting with {prefix:?} in section 9"))
            .to_owned()
    };
    let context = row("| Context snapshots");
    assert!(context.contains("| ~60 | ~4.5 KB |"), "{context}");
    let counters = row("| UI counters");
    assert!(counters.contains("| ~300 rows | ~7 KB |"), "{counters}");
    let dictionaries = row("| Dictionaries and sessions");
    assert!(
        dictionaries.contains("| ~20 KB while the library is being discovered"),
        "{dictionaries}"
    );
    for (prefix, events, bytes) in [
        ("| Play start/end", "~240", "~14 KB"),
        ("| Volume, mute, speed", "~60", "~4 KB"),
        ("| Seeks, replays", "~60", "~4 KB"),
        ("| Queue, favorites", "~40", "~2.5 KB"),
        ("| Search", "~20", "~1.5 KB"),
        ("| Output/EQ/window", "~30", "~2 KB"),
    ] {
        let line = row(prefix);
        assert!(line.contains(&format!("| {events} | {bytes} |")), "{line}");
    }
}
