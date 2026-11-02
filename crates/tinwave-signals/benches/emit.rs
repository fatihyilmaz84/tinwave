//! Producer-side cost of the signal bus.
//!
//! `docs/rules/testing.md` budget: signal emit (producer side) < 50 us. Criterion reports mean
//! and median, but the budget is a tail: after the criterion groups this binary measures every
//! call individually and **fails (non-zero exit) when the p99 is 50 us or more**, in three
//! scenarios: a draining writer, a low-priority flood that evicts on every call, and four
//! producer threads contending while the writer drains.
//!
//! Run: `cargo bench -p tinwave-signals --bench emit`.

// Benchmarks are test-like code; the library lints against unwrap/expect do not apply.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use criterion::{criterion_group, Criterion};
use tinwave_core::{DeviceId, RecordingId, SessionId, SignalEventId, SystemClock, UtcMillis};
use tinwave_signals::{
    BusConfig, CounterEntity, CounterRow, ElapsedDs, EntityKind, EventContext, EventStamp,
    GestureCoalescer, GestureTick, GestureTime, InputSource, PausedPayload, Section, SignalBus,
    SignalCategory, SignalDrain, SignalEvent, SignalKind, SignalPayload, SignalSettings,
    SignalSink, SignalsError, View, ViewOpenedPayload, Volume, VolumePayload,
};

/// The budget from `docs/rules/testing.md`.
const P99_BUDGET: Duration = Duration::from_micros(50);
const SAMPLES: usize = 200_000;

/// A sink that only counts, like the cheapest possible writer.
#[derive(Default)]
struct CountingSink {
    events: u64,
}

impl SignalSink for CountingSink {
    fn write(&mut self, events: &[SignalEvent], _: &[CounterRow]) -> Result<(), SignalsError> {
        self.events += events.len() as u64;
        Ok(())
    }
}

struct Ids {
    session: SessionId,
    recording: RecordingId,
}

fn make_ids() -> Ids {
    let at = UtcMillis::new(1_793_577_600_000);
    Ids {
        session: SessionId::from_parts(at, [2; 10]).unwrap(),
        recording: RecordingId::from_parts(at, [3; 10]).unwrap(),
    }
}

fn event(ids: &Ids, stamp: EventStamp, payload: SignalPayload) -> SignalEvent {
    SignalEvent {
        id: SignalEventId::new(stamp.id.device, stamp.id.seq),
        ts: stamp.ts,
        session: ids.session,
        recording: Some(ids.recording),
        source: None,
        pos_ds: Some(1_234),
        payload,
        ctx: None,
    }
}

fn paused() -> SignalPayload {
    SignalPayload::Paused(PausedPayload {
        input: InputSource::Mouse,
    })
}

fn view_opened() -> SignalPayload {
    SignalPayload::ViewOpened(ViewOpenedPayload { view: View::Lyrics })
}

fn new_bus() -> (SignalBus, SignalDrain) {
    let device = DeviceId::from_parts(UtcMillis::new(1_793_577_600_000), [1; 10]).unwrap();
    SignalBus::channel(
        BusConfig::new(device),
        SystemClock,
        Arc::new(SignalSettings::new()),
    )
    .unwrap()
}

/// A background writer: pumps whenever events arrive, until told to stop.
struct Writer {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<u64>>,
}

impl Writer {
    fn start(mut drain: SignalDrain) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            let mut sink = CountingSink::default();
            while !flag.load(Ordering::Relaxed) {
                if drain.wait(Duration::from_millis(5)) {
                    drain.pump(&mut sink).unwrap();
                }
            }
            sink.events
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

fn bench_emit(c: &mut Criterion) {
    let ids = make_ids();
    let mut group = c.benchmark_group("signal_emit");

    {
        let (bus, drain) = new_bus();
        let writer = Writer::start(drain);
        group.bench_function("reserved_event_with_draining_writer", |b| {
            b.iter(|| {
                bus.emit_with(SignalKind::Paused, |s| event(&ids, s, paused()))
                    .unwrap()
            });
        });
        black_box(writer.finish());
    }

    {
        // Nobody drains: after the first 512 calls every call evicts the oldest event.
        let (bus, _drain) = new_bus();
        group.bench_function("low_event_evicting_oldest", |b| {
            b.iter(|| {
                bus.emit_with(SignalKind::ViewOpened, |s| event(&ids, s, view_opened()))
                    .unwrap()
            });
        });
    }

    {
        let (bus, _drain) = new_bus();
        bus.settings()
            .set_category(SignalCategory::SeekSkipReplay, false);
        group.bench_function("disabled_category", |b| {
            b.iter(|| {
                bus.emit_with(SignalKind::Paused, |s| event(&ids, s, paused()))
                    .unwrap()
            });
        });
    }

    {
        let (bus, _drain) = new_bus();
        group.bench_function("ui_counter_impression", |b| {
            b.iter(|| {
                bus.counters()
                    .impression(black_box(CounterEntity::new(EntityKind::Track, [42; 16])))
            });
        });
    }

    {
        let (bus, drain) = new_bus();
        let writer = Writer::start(drain);
        let mut coalescer = GestureCoalescer::new(bus);
        let context = EventContext {
            session: ids.session,
            recording: Some(ids.recording),
            source: None,
            pos_ds: None,
            ctx: None,
        };
        group.bench_function("coalescer_volume_tick", |b| {
            let mut level = 0_u8;
            b.iter(|| {
                level = level.wrapping_add(1);
                coalescer.volume(|| GestureTick {
                    context,
                    payload: VolumePayload {
                        from: Volume::from_raw(level),
                        to: Volume::from_raw(level.wrapping_add(1)),
                        gesture: GestureTime::from_ms(0),
                        input: InputSource::Mouse,
                        section: Section::UNKNOWN,
                        since_track_start: ElapsedDs::from_ds(0),
                        since_section_start: ElapsedDs::from_ds(0),
                    },
                })
            });
        });
        drop(coalescer);
        black_box(writer.finish());
    }

    group.finish();
}

criterion_group!(benches, bench_emit);

/// Times every call and returns the sorted samples.
fn sample(mut call: impl FnMut()) -> Vec<Duration> {
    // Warm up so the first page faults are not in the tail.
    for _ in 0..10_000 {
        call();
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        call();
        samples.push(start.elapsed());
    }
    samples.sort_unstable();
    samples
}

fn check(scenario: &str, samples: &[Duration]) -> bool {
    let at = |q: f64| samples[((samples.len() as f64 * q) as usize).min(samples.len() - 1)];
    let (p50, p99, p999, max) = (at(0.50), at(0.99), at(0.999), samples[samples.len() - 1]);
    let ok = p99 < P99_BUDGET;
    println!(
        "p99 gate [{}] {scenario}: p50 {p50:?}  p99 {p99:?}  p99.9 {p999:?}  max {max:?}  (budget {P99_BUDGET:?})",
        if ok { "ok" } else { "FAIL" }
    );
    ok
}

fn p99_gate() -> bool {
    let ids = make_ids();
    let mut all_ok = true;

    {
        let (bus, drain) = new_bus();
        let writer = Writer::start(drain);
        let samples = sample(|| {
            let _ = black_box(bus.emit_with(SignalKind::Paused, |s| event(&ids, s, paused())));
        });
        all_ok &= check("reserved event, draining writer", &samples);
        black_box(writer.finish());
    }

    {
        let (bus, _drain) = new_bus();
        let samples = sample(|| {
            let _ =
                black_box(bus.emit_with(SignalKind::ViewOpened, |s| event(&ids, s, view_opened())));
        });
        all_ok &= check("low event, queue full (evicts every call)", &samples);
        assert!(bus.dropped(SignalKind::ViewOpened) > 0, "the flood evicted");
    }

    {
        let (bus, drain) = new_bus();
        let writer = Writer::start(drain);
        let stop = Arc::new(AtomicBool::new(false));
        let contenders: Vec<_> = (0..3)
            .map(|_| {
                let bus = bus.clone();
                let stop = stop.clone();
                let ids = make_ids();
                thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = bus
                            .emit_with(SignalKind::ViewOpened, |s| event(&ids, s, view_opened()));
                    }
                })
            })
            .collect();
        let samples = sample(|| {
            let _ = black_box(bus.emit_with(SignalKind::Paused, |s| event(&ids, s, paused())));
        });
        stop.store(true, Ordering::Relaxed);
        for c in contenders {
            c.join().unwrap();
        }
        all_ok &= check("reserved event, 3 low-priority flooders + writer", &samples);
        black_box(writer.finish());
        println!(
            "  (contention run: {} dropped low events, {} reserved overflows; a tight loop emits ~10^6 events/s, five orders above a real listener, so overflows here are expected and are timed too)",
            bus.dropped_total(),
            bus.reserved_overflows()
        );
    }

    all_ok
}

fn main() {
    benches();
    Criterion::default().configure_from_args().final_summary();
    if !p99_gate() {
        eprintln!("signal emit p99 exceeded the 50 us budget");
        std::process::exit(1);
    }
}
