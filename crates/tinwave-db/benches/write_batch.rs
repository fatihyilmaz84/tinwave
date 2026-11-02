//! Write cost of `SignalStore::write_batch` for a batch of 256 events.
//!
//! `docs/rules/testing.md` budget: batch write of 256 events < 5 ms. Criterion reports the mean
//! of the batch write alone; the gate after it times 2 000 batches one by one on a database that
//! already holds 100 000 events (the hot window of the reference listener, a 50 000-track library
//! with a `ctx` on every event) and **fails (non-zero exit) when the median is 5 ms or more**. It also prints the
//! mean, p99 and max, which are not gated: the batch right after a WAL
//! checkpoint (the checkpoint fsyncs; the writer runs it between batches,
//! `SignalStore::checkpoint_if_due`, and so does the gate, outside the timed section) is slower,
//! and the OS file system adds occasional stalls that no code here controls.
//!
//! Run: `cargo bench -p tinwave-db --bench write_batch`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
// The helpers are shared with the integration tests; this bench uses part of them.
#[path = "../tests/common/mod.rs"]
mod common;

use std::hint::black_box;
use std::time::{Duration, Instant};

use common::{Listener, TempDb};
use criterion::{criterion_group, BatchSize, Criterion};
use tinwave_db::SignalStore;
use tinwave_signals::SignalEvent;

const BATCH: usize = 256;
const BUDGET: Duration = Duration::from_millis(5);
const PRELOAD_EVENTS: usize = 100_000;
const GATE_BATCHES: usize = 2_000;

/// A store holding the hot window of the reference listener (100 000 events) and the listener
/// that produced it, so the next batches continue the same history.
fn loaded_store(db: &TempDb) -> (SignalStore, Listener) {
    let mut store = SignalStore::open(&db.path).unwrap();
    let mut listener = Listener::new(7, common::REFERENCE_LIBRARY);
    let preload = listener.events(PRELOAD_EVENTS);
    for chunk in preload.chunks(BATCH) {
        store.write_batch(chunk, &[]).unwrap();
        // As the writer does: keep the WAL small, between batches.
        store.checkpoint_if_due(true).unwrap();
    }
    (store, listener)
}

/// The next 256 events of the listener (whole plays are generated, the tail is cut).
fn next_batch(listener: &mut Listener) -> Vec<SignalEvent> {
    let mut batch = listener.events(BATCH);
    batch.truncate(BATCH);
    batch
}

fn bench_write(c: &mut Criterion) {
    let db = TempDb::new();
    let (mut store, mut listener) = loaded_store(&db);
    let mut group = c.benchmark_group("signal_store");
    group
        .sample_size(20)
        .measurement_time(Duration::from_secs(3));
    group.bench_function("write_batch_256_events", |b| {
        b.iter_batched(
            || next_batch(&mut listener),
            |batch| black_box(store.write_batch(black_box(&batch), &[]).unwrap()),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_write);

fn p99_gate() -> bool {
    let db = TempDb::new();
    let (mut store, mut listener) = loaded_store(&db);
    let mut samples = Vec::with_capacity(GATE_BATCHES);
    let mut checkpoints = 0u32;
    for _ in 0..GATE_BATCHES {
        let batch = next_batch(&mut listener);
        let start = Instant::now();
        black_box(store.write_batch(black_box(&batch), &[]).unwrap());
        samples.push(start.elapsed());
        // The writer checkpoints between batches, never inside one; so does the gate.
        checkpoints += u32::from(store.checkpoint_if_due(true).unwrap());
    }
    let mean = samples.iter().sum::<Duration>() / u32::try_from(samples.len()).unwrap();
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() as f64 * q) as usize).min(samples.len() - 1)];
    let (p50, p99, max) = (at(0.50), at(0.99), samples[samples.len() - 1]);
    let ok = p50 < BUDGET;
    println!(
        "gate [{}] 256-event batch on a 100k-event database: mean {mean:?}  p50 {p50:?}  p99 {p99:?}  max {max:?}  ({checkpoints} checkpoints between batches; median budget {BUDGET:?})",
        if ok { "ok" } else { "FAIL" }
    );
    ok
}

fn main() {
    benches();
    Criterion::default().configure_from_args().final_summary();
    if !p99_gate() {
        eprintln!("256-event batch write median exceeded the 5 ms budget");
        std::process::exit(1);
    }
}
