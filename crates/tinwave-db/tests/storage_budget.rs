//! The storage budget of `docs/rules/storage-budget.md` section 3: at most 60 bytes per raw
//! event on disk, table and indexes included, for 100 000 realistic events.
//!
//! Measured from SQLite's page accounting, not estimated: the `dbstat` virtual table gives the
//! pages of `signal_event` (the primary-key b-tree) and of its one index; the file's `page_count` gives the
//! total including the dictionaries and sessions.
//!
//! `signal_event` is `WITHOUT ROWID` keyed on `(device, seq)` with the single index
//! `(recording, session, ts_off)`: there is no rowid and no separate unique index.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{count, Listener, TempDb};
use tinwave_db::SignalStore;

const EVENTS: usize = 100_000;
const BATCH: usize = 256;
const BUDGET_BYTES_PER_EVENT: f64 = 60.0;
/// What the schema of `database.md` costs today (57.3 measured), with a little
/// headroom: a guard against the figure creeping up while the 60 B budget still passes.
const REGRESSION_CEILING_BYTES_PER_EVENT: f64 = 58.5;

/// Writes 100 000 realistic events in batches of 256 and returns bytes per event of the table
/// and its indexes.
fn measure() -> f64 {
    let db = TempDb::new();
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut listener = Listener::new(2026, 5_000);
    let events = listener.events(EVENTS);
    let total = i64::try_from(events.len()).unwrap();
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

    let pages = |name: &str| -> i64 {
        count(
            &r,
            &format!("SELECT count(*) FROM dbstat WHERE name = '{name}'"),
        )
    };
    let event_pages = pages("signal_event");
    let rec_ts_pages = pages("signal_event_rec_ts");
    let per_event = |p: i64| -> f64 { (p * page_size) as f64 / total as f64 };
    let measured = per_event(event_pages + rec_ts_pages);

    let file_pages = count(&r, "PRAGMA page_count") - count(&r, "PRAGMA freelist_count");
    let file_per_event = per_event(file_pages);
    println!(
        "{total} events: table {:.1} B/event, signal_event_rec_ts {:.1}; \
         table + indexes {measured:.1} B/event; whole file (dictionaries, sessions, \
         schema) {file_per_event:.1} B/event ({} KiB)",
        per_event(event_pages),
        per_event(rec_ts_pages),
        file_pages * page_size / 1024,
    );
    measured
}

#[test]
fn bytes_per_event_do_not_regress_beyond_the_measured_schema_cost() {
    let measured = measure();
    assert!(
        measured <= REGRESSION_CEILING_BYTES_PER_EVENT,
        "{measured:.1} bytes per event incl. index; was 57.3"
    );
}

#[test]
fn a_hundred_thousand_realistic_events_stay_within_sixty_bytes_each() {
    let measured = measure();
    assert!(
        measured <= BUDGET_BYTES_PER_EVENT,
        "{measured:.1} bytes per event incl. index exceeds the {BUDGET_BYTES_PER_EVENT} byte budget"
    );
}
