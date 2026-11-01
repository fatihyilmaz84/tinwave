//! The render callback performs zero heap allocations and zero frees (ADR 0005, testing.md).
//!
//! A counting global allocator counts every alloc / dealloc / realloc made by a thread while its
//! "in the callback" flag is set. The test thread plays the audio callback (through
//! `ManualHandle::render`), so the flag is raised exactly around each render call; the decoder
//! thread and the control calls are free to allocate and are not counted.
//!
//! This is the one place in the crate that needs `unsafe` (implementing `GlobalAlloc`).
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::*;

thread_local! {
    static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
    /// Per thread, so tests running in parallel cannot see each other's allocations.
    static CALLBACK_ALLOCS: Cell<u64> = const { Cell::new(0) };
}

struct CountingAlloc;

fn count() {
    // `try_with`: thread-local destruction can race with a late free; that is not the callback.
    if IN_CALLBACK.try_with(Cell::get).unwrap_or(false) {
        let _ = CALLBACK_ALLOCS.try_with(|n| n.set(n.get() + 1));
    }
}

// SAFETY: forwards every call unchanged to the system allocator; the counter is lock-free and
// never allocates.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        System.alloc_zeroed(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Runs one callback with the counting flag raised. Returns real frames rendered.
fn callback(h: &Harness, buf: &mut [f32]) -> usize {
    IN_CALLBACK.with(|c| c.set(true));
    let frames = h.out.render(buf, 480);
    IN_CALLBACK.with(|c| c.set(false));
    frames.expect("stream open")
}

fn allocs() -> u64 {
    CALLBACK_ALLOCS.with(Cell::get)
}

/// Guards against a vacuous pass: the counter must see allocations made while the flag is up.
#[test]
fn counter_detects_allocation() {
    let before = allocs();
    IN_CALLBACK.with(|c| c.set(true));
    let v: Vec<u8> = Vec::with_capacity(64);
    drop(v);
    IN_CALLBACK.with(|c| c.set(false));
    assert!(allocs() >= before + 2, "alloc + free must both count");
}

#[test]
fn render_never_allocates_across_play_seek_volume_pause_and_device_change() {
    let dir = TempDir::new("rt-alloc");
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);

    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    let mut buf = vec![0.0f32; 2 * 441];
    let mut real_frames = 0usize;
    let mut starved_callbacks = 0usize;
    let mut run = |h: &mut Harness, buf: &mut [f32], callbacks: usize, wait_for_data: bool| {
        for _ in 0..callbacks {
            if wait_for_data {
                h.wait_buffered(441);
            }
            let n = callback(h, buf);
            real_frames += n;
            starved_callbacks += usize::from(n == 0);
            h.engine.poll();
        }
    };

    // Same as `Harness::settle`, but every callback it makes is counted too.
    let settle = |h: &mut Harness, buf: &mut [f32]| {
        for _ in 0..30_000 {
            if !h.engine.command_pending() {
                return;
            }
            callback(h, buf);
            h.engine.poll();
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder never carried out the command");
    };

    let start = allocs();

    // Command handshake and first fill: callbacks race the decoder, so some are starved
    // (underrun path) -- they must not allocate either.
    run(&mut h, &mut buf, 40, false);
    settle(&mut h, &mut buf);
    // Play.
    run(&mut h, &mut buf, 30, true);
    // Volume changes: ramp down, hold, ramp up.
    h.engine.set_volume(0.25).expect("volume");
    run(&mut h, &mut buf, 5, true);
    h.engine.set_volume(1.0).expect("volume");
    run(&mut h, &mut buf, 5, true);
    // Pause and resume (ramped, then silent callbacks).
    h.engine.pause();
    run(&mut h, &mut buf, 5, false);
    h.engine.resume();
    run(&mut h, &mut buf, 5, true);
    // Seek: flush handshake, then refill.
    h.engine.seek(20_001).expect("seek");
    run(&mut h, &mut buf, 20, false);
    settle(&mut h, &mut buf);
    run(&mut h, &mut buf, 20, true);
    // Device change: new ring, new renderer.
    h.engine
        .switch_device(Some("other".into()))
        .expect("switch");
    run(&mut h, &mut buf, 20, false);
    settle(&mut h, &mut buf);
    run(&mut h, &mut buf, 20, true);
    // Cross the track boundary and run to the end of the queue and past it.
    h.engine.seek(47_500).expect("seek");
    settle(&mut h, &mut buf);
    run(&mut h, &mut buf, 80, true);
    // Load near the end of B: the stream ends mid-run and the callbacks keep coming.
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 1, 47_000)
        .expect("load");
    settle(&mut h, &mut buf);
    run(&mut h, &mut buf, 60, false);

    let during = allocs() - start;
    assert_eq!(
        during, 0,
        "the render callback allocated or freed {during} times"
    );
    assert!(
        real_frames > 20_000,
        "test must actually render audio, got {real_frames} frames"
    );
    assert!(
        starved_callbacks > 0,
        "test must also cover callbacks with nothing to play"
    );
}
