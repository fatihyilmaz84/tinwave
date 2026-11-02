//! Extra resident memory of the signals.db writer.
//!
//! `docs/rules/testing.md` budget: writer extra RSS < 2 MB. This runs in its own process (no
//! criterion, no test harness) so the numbers are not polluted: it builds the bus and holds
//! 100 000 pre-generated events, takes the baseline RSS, then opens the store, starts the writer
//! thread, pushes all events (and some UI counters) through the bus into a real database, and
//! reads the RSS again, sampling during the run for the peak. **Fails (non-zero exit) when the
//! growth is 2 MB or more.**
//!
//! RSS is read with `tasklist` (Windows) or `ps` (Linux, macOS): this crate forbids `unsafe`, so
//! no OS API is called directly. Resolution is 1 KiB (Windows working set).
//!
//! Run: `cargo bench -p tinwave-db --bench writer_rss`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../tests/common/mod.rs"]
mod common;

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Listener, TempDb};
use tinwave_core::SystemClock;
use tinwave_db::{SignalStore, SignalWriter, WriterConfig};
use tinwave_signals::{BusConfig, CounterEntity, EntityKind, SignalBus, SignalSettings};

const EVENTS: usize = 100_000;
const CHUNK: usize = 1_000;
const BUDGET_KIB: i64 = 2 * 1024;

/// Resident set (working set on Windows) of this process, in KiB.
fn rss_kib() -> i64 {
    let pid = std::process::id();
    if cfg!(windows) {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .expect("run tasklist");
        let text = String::from_utf8_lossy(&out.stdout);
        // One CSV row ending in the memory column, which reads like "12,345 K": the thousands
        // separator is a comma, so split on the quote-comma-quote between fields instead.
        let mem = text.trim().rsplit("\",\"").next().expect("a memory column");
        mem.chars()
            .filter(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .expect("a number")
    } else {
        let out = Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .expect("run ps");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .expect("a number")
    }
}

fn main() {
    let db = TempDb::new();
    let mut listener = Listener::new(99, 5_000);
    let device = listener.device();
    let events = listener.events(EVENTS);
    let (bus, drain) = SignalBus::channel(
        BusConfig::new(device),
        SystemClock,
        Arc::new(SignalSettings::new()),
    )
    .unwrap();
    let album = CounterEntity::new(EntityKind::Album, [5; 16]);
    // Warm the RSS reader once so its one-off costs are in the baseline.
    let _ = rss_kib();
    let cold_baseline = rss_kib();
    // The code of SQLite itself is in the process once anything opens a database (library.db is
    // open long before the first signal), so it is not the writer's memory: touch it first.
    {
        let warm = rusqlite::Connection::open_in_memory().unwrap();
        warm.execute_batch("CREATE TABLE t (a INTEGER PRIMARY KEY, b BLOB, UNIQUE (b)); BEGIN;")
            .unwrap();
        for i in 0..2_000i64 {
            warm.execute("INSERT INTO t (b) VALUES (?1)", [i.to_le_bytes().to_vec()])
                .unwrap();
        }
        warm.execute_batch("COMMIT;").unwrap();
        let n: i64 = warm
            .query_row("SELECT count(*) FROM t WHERE b > x'00'", [], |r| r.get(0))
            .unwrap();
        assert!(n > 0);
    }
    let baseline = rss_kib();

    let store = SignalStore::open(&db.path).unwrap();
    let writer = SignalWriter::spawn(store, drain, WriterConfig::default()).unwrap();
    let mut peak = baseline;
    let mut sent = 0u64;
    for (i, chunk) in events.chunks(CHUNK).enumerate() {
        for event in chunk {
            let ev = *event;
            let _ = bus
                .emit_with(ev.kind(), |stamp| {
                    let mut e = ev;
                    e.id = stamp.id;
                    e
                })
                .unwrap();
            sent += 1;
        }
        let _ = bus.counters().impression(album);
        // Stay below the queue capacity: wait until the writer has taken most of the chunk.
        let start = Instant::now();
        while bus.queued_total() - writer.stats().events_inserted > CHUNK as u64 / 2 {
            assert!(start.elapsed() < Duration::from_secs(30), "writer stalled");
            std::thread::sleep(Duration::from_millis(2));
        }
        if i % 25 == 24 {
            peak = peak.max(rss_kib());
        }
    }
    let steady = rss_kib();
    peak = peak.max(steady);
    let stats = writer.shutdown().unwrap();
    assert_eq!(
        stats.events_inserted, sent,
        "every event reached the database"
    );
    assert_eq!(bus.dropped_total(), 0);
    let after = rss_kib();

    let growth = peak - baseline;
    println!(
        "writer RSS: cold baseline {cold_baseline} KiB, baseline with SQLite warm {baseline} KiB, steady {steady} KiB (+{} KiB), peak sampled {peak} KiB (+{growth} KiB), after shutdown {after} KiB; {} events in {} transactions; budget {BUDGET_KIB} KiB: {}",
        steady - baseline,
        stats.events_inserted,
        stats.batches_committed,
        if growth < BUDGET_KIB { "ok" } else { "FAIL" }
    );
    if growth >= BUDGET_KIB {
        std::process::exit(1);
    }
}
