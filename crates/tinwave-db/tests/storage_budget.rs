//! The storage budget of `docs/rules/storage-budget.md` section 3: at most 60 bytes per raw
//! event on disk, table and indexes included, for 100 000 realistic events.
//!
//! "Realistic" is the reference heavy user of `storage-budget.md` / `testing.md`: a 50 000-track
//! library, a few sessions a day, the event mix of `common::Listener` (about 6.4 events per play),
//! and a context snapshot id (`ctx`) on every event, a new one about every 8 events.
//!
//! Measured from SQLite's page accounting, not estimated: the `dbstat` virtual table gives the
//! pages of `signal_event`; the file's `page_count` gives the total including the dictionaries,
//! sessions and the schema.
//!
//! `signal_event` is `WITHOUT ROWID` keyed on `(device, seq)` and has no secondary index
//! (`docs/rules/database.md`), so the table is the whole cost of an event.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::OnceLock;

use common::{count, Listener, TempDb, REFERENCE_LIBRARY};
use tinwave_db::SignalStore;

const EVENTS: usize = 100_000;
const BATCH: usize = 256;
const BUDGET_BYTES_PER_EVENT: f64 = 60.0;
/// What the schema of `database.md` costs today (39.4 B/event measured), with a
/// little headroom: a guard against the figure creeping up while the 60 B budget still passes.
const REGRESSION_CEILING_BYTES_PER_EVENT: f64 = 41.0;

#[derive(Debug, Clone, Copy)]
struct Measured {
    /// Bytes per event of the `signal_event` table (and its indexes: there are none).
    table: f64,
    /// Bytes per event of the whole file, dictionaries and sessions included.
    file: f64,
}

/// Writes 100 000 realistic events in batches of 256 and measures the file. Both tests share
/// one run.
fn measure() -> Measured {
    static MEASURED: OnceLock<Measured> = OnceLock::new();
    *MEASURED.get_or_init(run)
}

fn run() -> Measured {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut listener = Listener::new(2026, REFERENCE_LIBRARY);
    let events = listener.events(EVENTS);
    let total = i64::try_from(events.len()).unwrap();
    assert!(
        events.iter().all(|e| e.ctx.is_some()),
        "every event has a ctx"
    );
    for batch in events.chunks(BATCH) {
        store.write_batch(batch, &[]).unwrap();
    }
    drop(store);

    // Fold the WAL into the main file so the page counts are the file's.
    let r = db.reader();
    r.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    let page_size = count(&r, "PRAGMA page_size");
    assert_eq!(count(&r, "SELECT count(*) FROM signal_event"), total);
    assert_eq!(
        count(
            &r,
            "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'signal_event' \
             AND sql IS NOT NULL"
        ),
        0,
        "signal_event has no secondary index"
    );

    let table_pages = count(
        &r,
        "SELECT count(*) FROM dbstat WHERE name = 'signal_event'",
    );
    let per_event = |p: i64| -> f64 { (p * page_size) as f64 / total as f64 };
    let file_pages = count(&r, "PRAGMA page_count") - count(&r, "PRAGMA freelist_count");
    let measured = Measured {
        table: per_event(table_pages),
        file: per_event(file_pages),
    };
    println!(
        "{total} events ({:.2} per play, library {REFERENCE_LIBRARY}): signal_event table \
         {:.1} B/event (no index); whole file (dictionaries, sessions, schema) {:.1} B/event \
         ({} KiB)",
        listener.events_per_play(),
        measured.table,
        measured.file,
        file_pages * page_size / 1024,
    );
    measured
}

#[test]
fn bytes_per_event_do_not_regress_beyond_the_measured_schema_cost() {
    let measured = measure().table;
    assert!(
        measured <= REGRESSION_CEILING_BYTES_PER_EVENT,
        "{measured:.1} bytes per event; was 39.4"
    );
}

#[test]
fn a_hundred_thousand_realistic_events_stay_within_sixty_bytes_each() {
    let measured = measure().table;
    assert!(
        measured <= BUDGET_BYTES_PER_EVENT,
        "{measured:.1} bytes per event exceeds the {BUDGET_BYTES_PER_EVENT} byte budget"
    );
}
