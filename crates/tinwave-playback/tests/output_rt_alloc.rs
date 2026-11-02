//! The output callback allocates nothing: zero heap allocations and frees across everything one
//! WASAPI wake-up does (`Pump::step`: wait, query room, render, convert to the device format,
//! write), in every device format, across play, volume, pause, seek, device change and the end of
//! the queue.
//!
//! The device calls themselves are the fake's (preallocated `Vec` that is cleared, never grown),
//! so what is counted is this crate's callback code. The real WASAPI calls are COM method calls
//! with no Rust allocation of ours on the success path (`write_to_device` is a bounds check and a
//! `copy_from_slice`, read in the `wasapi` crate); the opt-in device test exercises them live.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::Arc;

use common::*;
use tinwave_core::{ManualClock, UtcMillis};
use tinwave_playback::{Engine, EngineConfig, FsOpener, PumpStep, SampleFormat};
use tinwave_test_alloc::{count_allocations, CountingAlloc};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

const BLOCK: usize = 480;

struct Rig {
    engine: Engine,
    dev: FakeDeviceHandle,
    allocs: u64,
    real_frames: usize,
    steps: usize,
}

impl Rig {
    fn run(&mut self, wakeups: usize, wait_for_data: bool) {
        for _ in 0..wakeups {
            if wait_for_data {
                for _ in 0..30_000 {
                    if self.dev.buffered_frames() >= BLOCK {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            }
            let (step, allocs) = count_allocations(|| self.dev.step_discarding());
            self.allocs += allocs;
            self.steps += 1;
            if let Some(PumpStep::Fed { real, .. }) = step {
                self.real_frames += real;
            }
            self.engine.poll();
        }
    }

    fn settle(&mut self) {
        for _ in 0..30_000 {
            if !self.engine.command_pending() {
                return;
            }
            let (_, allocs) = count_allocations(|| self.dev.service_flush());
            self.allocs += allocs;
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder never carried out the command");
    }
}

fn run_format(format: SampleFormat) {
    let dir = TempDir::new(&format!("output-rt-alloc-{format:?}"));
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);

    let (output, dev) = FakeDeviceOutput::new(format, BLOCK);
    let engine = Engine::start(
        EngineConfig {
            sample_rate: RATE,
            device: None,
            opener: Arc::new(FsOpener),
            sink: Arc::new(CollectingSink::default()),
            clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
        },
        Box::new(output),
    )
    .expect("engine starts");
    let mut rig = Rig {
        engine,
        dev,
        allocs: 0,
        real_frames: 0,
        steps: 0,
    };
    rig.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");

    rig.run(30, false); // wakeups race the decoder: starved ones must not allocate either
    rig.settle();
    rig.run(30, true); // play
    rig.engine.set_volume(0.25).expect("volume");
    rig.run(5, true);
    rig.engine.set_volume(1.0).expect("volume");
    rig.run(5, true);
    rig.engine.pause();
    rig.run(5, false);
    rig.engine.resume();
    rig.run(5, true);
    rig.engine.seek(20_001).expect("seek");
    rig.run(10, false);
    rig.settle();
    rig.run(20, true);
    rig.engine
        .switch_device(Some("other".into()))
        .expect("switch"); // builds a new pump: control thread, not counted
    rig.run(10, false);
    rig.settle();
    rig.run(20, true);
    rig.engine.seek(47_500).expect("seek"); // across the A|B boundary
    rig.settle();
    rig.run(60, true);
    rig.engine
        .load(vec![track(1, &a), track(2, &b)], 1, 47_000)
        .expect("load");
    rig.settle();
    rig.run(40, false); // runs into the end of the queue and past it

    assert_eq!(
        rig.allocs, 0,
        "{format:?}: the output callback allocated or freed {} times",
        rig.allocs
    );
    assert!(
        rig.real_frames > 20_000,
        "{format:?}: the test must render audio, got {} frames",
        rig.real_frames
    );
}

#[test]
fn the_output_callback_never_allocates_in_any_device_format() {
    for format in [
        SampleFormat::F32,
        SampleFormat::I16,
        SampleFormat::I24In32,
        SampleFormat::I32,
    ] {
        run_format(format);
    }
}

/// Guard against a vacuous pass: allocation inside the same counted region is seen.
#[test]
fn counter_sees_an_allocation_in_the_counted_region() {
    let (_, allocs) = count_allocations(|| drop(std::hint::black_box(Vec::<u8>::with_capacity(8))));
    assert_eq!(allocs, 2);
}
