//! Storage growth test v1 (ADR 0002, `docs/rules/testing.md`): a synthetic heavy listener, driven
//! through the real bus, coalescers, counters and `SignalWriter` into `signals.db`, measured against
//! the budgets and the catalog's volume table.
//!
//! Default run (fast): the P-0010 burst assertion and a 3-day determinism run.
//!
//! The 90-day run takes about 15 s of wall clock (the writer's commits, not the generator, are the
//! cost), so it is in the default run. The writer's CPU time for it is about 6 s in a debug build
//! and about 3 s in a release build (the generator and the libraries are optimized in release;
//! SQLite is optimized in both, which is why debug is only 2x slower, not more).
//!
//! The speed check is on the writer thread's CPU time, not the wall clock, so a busy machine cannot
//! fail it, and it has a separate budget per build profile ([`WRITER_CPU_BUDGET`]). It measures
//! the writer's CPU work only (batching, SQL, SQLite's own computing): a slowdown that costs
//! waiting rather than computing (an fsync per commit, a sleep, checkpoint I/O) does not move it.
//! Those are covered elsewhere: the pragma test
//! (`connection::tests::a_new_database_gets_every_pragma_of_database_md`) for `synchronous`, and
//! the `signal_writer` timing tests for latency. The harness raises the checkpoint limits, so
//! checkpoint cost is not measured here either. A hang is caught by the barrier's 60 s assert and,
//! for the whole binary, by the `terminate-after` override in `.config/nextest.toml`.
//! Its report is printed with `--nocapture`:
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

use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use common::{Ids, TempDb};
use cpu_time::{ProcessTime, ThreadTime};
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

/// `(seq, session, ts_off, kind, recording, payload)` of a stored event. `recording` is the
/// dictionary id, so a different id assignment shows up.
type EventRow = (i64, i64, i64, i64, i64, Option<Vec<u8>>);
/// `(day, kind, entity_kind, entity, count, total_ms, max_scrolled)` of a `ui_counter` row; `entity`
/// is a dictionary id too.
type CounterRow = (i64, i64, i64, i64, i64, i64, i64);

fn stored_rows(db: &TempDb) -> (Vec<EventRow>, Vec<CounterRow>) {
    let conn = Connection::open(&db.path).unwrap();
    let events = {
        let mut stmt = conn
            .prepare(
                "SELECT seq, session, ts_off, kind, coalesce(recording, 0), payload \
                 FROM signal_event ORDER BY device, seq",
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
    };
    let counters = {
        let mut stmt = conn
            .prepare(
                "SELECT day, kind, entity_kind, entity, count, total_ms, max_scrolled \
                 FROM ui_counter ORDER BY day, kind, entity_kind, entity",
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
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    };
    (events, counters)
}

/// Two runs stored the same thing: the same transactions, sizes and rows, including every
/// dictionary id.
fn assert_identical(a: &growth::Run, b: &growth::Run) {
    let (ra, rb) = (measure(a), measure(b));
    assert_eq!(a.writer.batches_committed, b.writer.batches_committed);
    assert_eq!(ra.file_bytes, rb.file_bytes);
    assert_eq!(ra.tables, rb.tables);
    assert_eq!(ra.events, rb.events);
    assert_eq!(ra.counter_rows, rb.counter_rows);
    assert_eq!(ra.context_rows, rb.context_rows);
    let (events_a, counters_a) = stored_rows(&a.db);
    let (events_b, counters_b) = stored_rows(&b.db);
    assert_eq!(events_a, events_b);
    assert_eq!(counters_a, counters_b);
}

/// Same seed, same data: two runs produce identical tables, sizes, rows and dictionary ids.
#[test]
fn the_same_seed_gives_the_same_sizes() {
    let cfg = GrowthConfig::reference(3);
    let a = run_alone(cfg);
    let b = run_alone(cfg);
    assert_identical(&a, &b);
    // A different seed is different data.
    let other = run_alone(GrowthConfig { seed: 7, ..cfg });
    assert_ne!(measure(&other).file_bytes, measure(&a).file_bytes);

    // And the run was clean: nothing dropped, nothing failed.
    for r in [&a, &b] {
        assert_eq!((r.dropped, r.reserved_overflows, r.keys_dropped), (0, 0, 0));
        assert_eq!(r.writer.failed_commits, 0);
        assert_eq!(r.writer.events_rejected, 0);
    }
}

/// The lock step does not depend on thread scheduling. `Paused` ends a play, so the writer
/// commits as soon as it pulls one; with the producer stalled 3 ms after each `Paused` (a
/// preempted thread) the writer would have pulled it alone, where an undisturbed run has pulled
/// the following events too. The harness waits for the commit after every play-end event, so both
/// runs cut their transactions at the same events: identical batches, rows and dictionary ids.
#[test]
fn the_rows_are_the_same_when_the_producer_is_preempted_after_a_pause() {
    let cfg = GrowthConfig::reference(5);
    let calm = run_alone(cfg);
    let preempted = run_alone(GrowthConfig {
        preempt_after_pause: true,
        ..cfg
    });
    // The test would prove nothing without pauses in the run.
    let (events, _) = stored_rows(&calm.db);
    let pauses = events
        .iter()
        .filter(|e| e.3 == i64::from(SignalKind::Paused.discriminant()))
        .count();
    assert!(pauses >= 10, "only {pauses} pauses in 5 days");
    assert_identical(&calm, &preempted);
}

fn assert_no_violations(violations: &[String]) {
    assert!(violations.is_empty(), "\n{}", violations.join("\n"));
}

/// Simulations running one at a time: the speed check reads the process's CPU time, which counts
/// every thread of the test binary, so no other simulation may overlap the 90-day run. (Under
/// `cargo nextest` every test is its own process and the lock is free; under `cargo test` the
/// small runs just queue behind it.)
static SIMULATION: Mutex<()> = Mutex::new(());

/// `run` with nothing else simulating in this process.
fn run_alone(cfg: GrowthConfig) -> growth::Run {
    let _alone = SIMULATION.lock().unwrap_or_else(PoisonError::into_inner);
    run(cfg, &mut |_| {})
}

/// The 90-day run, its report and what the writer cost in CPU time.
struct NinetyDays {
    run: growth::Run,
    report: growth::Report,
    /// CPU time the writer thread (batching, SQL, SQLite) spent during the run. See
    /// [`writer_cpu`].
    writer_cpu: Duration,
}

/// CPU time the writer thread spent while `body` ran on this thread.
///
/// The speed check exists to catch a slowdown of the writer path, and the wall clock cannot: it
/// stretches 2-3x when other processes (a parallel test run) compete for the cores. CPU time does
/// not. The process total counts every thread of the test binary; the generator thread (this one)
/// is read separately, and its share includes the time it spins waiting for the writer at the
/// barriers (sleeping, not spinning, so it is small), is subtracted and not asserted. What is
/// left is the writer thread, the only other thread running while `SIMULATION` is held.
fn writer_cpu<T>(body: impl FnOnce() -> T) -> (T, Duration, Duration) {
    let (process_start, thread_start) = (ProcessTime::now(), ThreadTime::now());
    let result = body();
    let (process, generator) = (process_start.elapsed(), thread_start.elapsed());
    (result, process.saturating_sub(generator), generator)
}

/// The writer's CPU time for the 90-day run depends on the build profile: about 6 s in a debug
/// build (6.0 to 6.5 s, the same under load while the wall clock went from 16 s to 59 s) and
/// about 3.2 s in a release build (3.1 to 3.5 s). Each budget is about 2.4x its measured value:
/// room for another machine, and a slowdown of 3x or more fails in either profile. (One budget
/// of 15 s would let a release run slow down 4.7x.) `cargo test --release` has
/// `debug_assertions` off; a profile that sets it differently gets the matching budget.
///
/// CPU work only: waiting (fsync, sleeps, checkpoint I/O) is not counted; see the module doc.
const WRITER_CPU_BUDGET: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(15)
} else {
    Duration::from_secs(8)
};

/// The 90-day run, shared by the tests below (one run, one report).
fn ninety_days() -> &'static NinetyDays {
    static RUN: OnceLock<NinetyDays> = OnceLock::new();
    RUN.get_or_init(|| {
        let _alone = SIMULATION.lock().unwrap_or_else(PoisonError::into_inner);
        let (run, writer_cpu, generator_cpu) =
            writer_cpu(|| run(GrowthConfig::reference(90), &mut |_| {}));
        let report = measure(&run);
        println!("{}", report.render(&run));
        println!(
            "speed: writer thread {:.1} s of CPU time (budget {:.0} s); generator thread {:.1} s of CPU time; {:.1} s of wall clock, not asserted (it grows with the machine's load)",
            writer_cpu.as_secs_f64(),
            WRITER_CPU_BUDGET.as_secs_f64(),
            generator_cpu.as_secs_f64(),
            run.elapsed.as_secs_f64()
        );
        NinetyDays {
            run,
            report,
            writer_cpu,
        }
    })
}

/// 90 simulated days of the reference heavy user: nothing may be lost on the way, the hot
/// window must fit 6 MB, the 5-year extrapolation must fit 60 MB, and the writer must stay
/// within its CPU budget.
#[test]
fn ninety_days_of_a_heavy_listener_fit_the_hot_window_and_the_five_year_budget() {
    let NinetyDays {
        run: r,
        report,
        writer_cpu,
    } = ninety_days();
    assert_eq!((r.dropped, r.reserved_overflows, r.keys_dropped), (0, 0, 0));
    assert_eq!((r.writer.failed_commits, r.writer.events_lost), (0, 0));
    let violations = report.budget_violations();
    assert_no_violations(&violations);
    assert!(
        *writer_cpu < WRITER_CPU_BUDGET,
        "the writer used {writer_cpu:?} of CPU time, budget {WRITER_CPU_BUDGET:?}"
    );
}

/// The same run against the volume table of `signal-catalog.md` section 9: every event group's
/// and the UI counters' bytes/day within +50 % of the catalog, and the generator within 25 % of
/// the catalog's rates.
#[test]
fn ninety_days_match_the_catalog_volume_table_within_fifty_percent() {
    let NinetyDays { report, .. } = ninety_days();
    let violations = report.catalog_violations();
    assert_no_violations(&violations);
}

/// Context snapshots against the catalog's "~60 a day, ~4.5 KB" (about 70 B per row, the 64-bit
/// hash with its UNIQUE index and the key included): rate within 25%, bytes/day within +50%.
#[test]
fn ninety_days_context_snapshots_match_the_catalog_estimate() {
    let NinetyDays { report, .. } = ninety_days();
    let violations = report.context_violations();
    assert_no_violations(&violations);
}

/// "~14 KB", "~2.5 KB": how the catalog writes a byte count (1 KB = 1000 B).
fn catalog_kb(bytes: f64) -> String {
    let kb = bytes / 1000.0;
    if kb.fract() == 0.0 {
        format!("~{kb:.0} KB")
    } else {
        format!("~{kb:.1} KB")
    }
}

/// The harness's expected numbers are the catalog's; this builds the text the catalog must contain
/// from the harness's constants, so editing the catalog or a constant fails here.
#[test]
fn the_catalog_still_says_what_the_test_expects() {
    let catalog = include_str!("../../../docs/guides/signal-catalog.md").replace("\r\n", "\n");
    let row = |prefix: &str| -> String {
        catalog
            .lines()
            .find(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("no row starting with {prefix:?} in section 9"))
            .to_owned()
    };
    let (ctx_rows, ctx_bytes) = growth::CATALOG_CONTEXT;
    let context = row("| Context snapshots");
    let expected = format!("| ~{ctx_rows:.0} | {} |", catalog_kb(ctx_bytes));
    assert!(
        context.contains(&expected),
        "expected {expected:?} in {context}"
    );

    let (ctr_rows, ctr_bytes) = growth::CATALOG_COUNTERS;
    let counters = row("| UI counters");
    let expected = format!("| ~{ctr_rows:.0} rows | {} |", catalog_kb(ctr_bytes));
    assert!(
        counters.contains(&expected),
        "expected {expected:?} in {counters}"
    );

    let dictionaries = row("| Dictionaries and sessions");
    let expected = format!(
        "| {} while the library is being discovered",
        catalog_kb(growth::CATALOG_DICTIONARIES_BYTES)
    );
    assert!(
        dictionaries.contains(&expected),
        "expected {expected:?} in {dictionaries}"
    );

    for group in growth::Group::ALL {
        let (Some(prefix), Some((events, bytes))) = (group.catalog_row_prefix(), group.catalog())
        else {
            continue;
        };
        let line = row(prefix);
        let expected = format!("| ~{events:.0} | {} |", catalog_kb(bytes));
        assert!(line.contains(&expected), "expected {expected:?} in {line}");
    }
}
