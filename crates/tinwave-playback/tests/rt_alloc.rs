//! The render callback performs zero heap allocations and zero frees (ADR 0005, testing.md).
//!
//! `tinwave_test_alloc::CountingAlloc` counts every alloc / dealloc / realloc made by the calling
//! thread inside `count_allocations`. The test thread plays the audio callback (through
//! `ManualHandle::render`), so the counted region is exactly each render call; the decoder thread
//! and the control calls are free to allocate and are not counted.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use common::*;
use tinwave_playback::QueueItemId;
use tinwave_test_alloc::{count_allocations, CountingAlloc};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Runs one callback (an empty buffer is the flush-acknowledgement callback) and returns the real
/// frames rendered and the allocations it made.
fn callback(h: &Harness, buf: &mut [f32]) -> (usize, u64) {
    let (frames, allocs) = count_allocations(|| h.out.render(buf, 480));
    (frames.expect("stream open"), allocs)
}

/// The test thread as audio callback, with running totals.
struct Rig {
    h: Harness,
    buf: Vec<f32>,
    allocs: u64,
    real_frames: usize,
    starved_callbacks: usize,
}

impl Rig {
    /// `callbacks` callbacks of 441 frames; with `wait_for_data` each waits until the ring holds a
    /// full block, without it the callbacks race the decoder and some are starved.
    fn run(&mut self, callbacks: usize, wait_for_data: bool) {
        for _ in 0..callbacks {
            if wait_for_data {
                self.h.wait_buffered(441);
            }
            let (n, a) = callback(&self.h, &mut self.buf);
            self.allocs += a;
            self.real_frames += n;
            self.starved_callbacks += usize::from(n == 0);
            self.h.engine.poll();
        }
    }

    /// Same as `Harness::settle`, with every acknowledgement callback counted too.
    fn settle(&mut self) {
        for _ in 0..30_000 {
            if !self.h.engine.command_pending() {
                return;
            }
            let (_, a) = callback(&self.h, &mut []);
            self.allocs += a;
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder never carried out the command");
    }
}

#[test]
fn render_never_allocates_across_play_seek_volume_pause_and_device_change() {
    let dir = TempDir::new("rt-alloc");
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);

    let mut rig = Rig {
        h: Harness::new(),
        buf: vec![0.0f32; 2 * 441],
        allocs: 0,
        real_frames: 0,
        starved_callbacks: 0,
    };
    rig.h
        .engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");

    // Command handshake (empty acknowledgement callbacks).
    rig.settle();
    // Queue edits: the tail is replaced twice. Nothing has been played, so the ring is full with
    // the first 48,000 of A's 48,777 frames and the decoder cannot have opened B yet: an edit
    // on A is certain to be applied (it is refused once the decoder moved on), which the test
    // checks, so the edit path really ran next to the callback.
    let edit = rig
        .h
        .engine
        .replace_after(QueueItemId(1), vec![track(3, &b), track(4, &a)])
        .expect("play next");
    rig.h.expect_edit_applied(edit);
    let edit = rig
        .h
        .engine
        .replace_after(QueueItemId(1), vec![track(2, &b)])
        .expect("remove");
    rig.h.expect_edit_applied(edit);
    // First fill: callbacks race the decoder, so some are starved (underrun path) -- they must
    // not allocate either.
    rig.run(40, false);
    // Play.
    rig.run(30, true);
    // Volume changes: ramp down, hold, ramp up.
    rig.h.engine.set_volume(0.25).expect("volume");
    rig.run(5, true);
    rig.h.engine.set_volume(1.0).expect("volume");
    rig.run(5, true);
    // Pause and resume (ramped, then silent callbacks).
    rig.h.engine.pause();
    rig.run(5, false);
    rig.h.engine.resume();
    rig.run(5, true);
    // Seek: flush handshake, then refill.
    rig.h.engine.seek(20_001).expect("seek");
    rig.run(20, false);
    rig.settle();
    rig.run(20, true);
    // Device change: new ring, new renderer.
    rig.h
        .engine
        .switch_device(Some("other".into()))
        .expect("switch");
    rig.run(20, false);
    rig.settle();
    rig.run(20, true);
    // Cross the track boundary and run to the end of the queue and past it.
    rig.h.engine.seek(47_500).expect("seek");
    rig.settle();
    rig.run(80, true);
    // Load near the end of B: the stream ends mid-run and the callbacks keep coming.
    rig.h
        .engine
        .load(vec![track(1, &a), track(2, &b)], 1, 47_000)
        .expect("load");
    rig.settle();
    rig.run(60, false);

    assert_eq!(
        rig.allocs, 0,
        "the render callback allocated or freed {} times",
        rig.allocs
    );
    assert!(
        rig.real_frames > 20_000,
        "test must actually render audio, got {} frames",
        rig.real_frames
    );
    assert!(
        rig.starved_callbacks > 0,
        "test must also cover callbacks with nothing to play"
    );
}

/// Guards against a vacuous pass: allocations made through the same counting path are seen.
#[test]
fn counter_detects_allocation_in_a_callback_shaped_region() {
    let h = Harness::new();
    let (_, allocs) = count_allocations(|| {
        let v: Vec<f32> = Vec::with_capacity(64);
        drop(v);
        h.out.buffered_frames()
    });
    assert_eq!(allocs, 2, "one allocation and one free");
}

/// The same guarantee when the decoder is resampling: a queue edit, a device change and a seek
/// inside a resampled track, then rate changes at every track boundary (44.1 kHz -> 48 kHz ->
/// 44.1 kHz -> 96 kHz -> 48 kHz). All resampling happens on the decoder thread, so the callback
/// still touches no heap.
#[test]
fn render_never_allocates_across_rate_changes_and_resampled_seeks() {
    let dir = TempDir::new("rt-alloc-rates");
    let [a, b, c] = rate_change_files(&dir);
    // 80,000 frames at 44.1 kHz, about 87,000 stream frames: far more than the ring holds plus
    // what this test plays before the edit, so the decoder is still reading it then.
    let long = dir.path("long44.wav");
    write_sweep_piece(&long, 44_100, 0, 80_000, 2);

    let mut rig = Rig {
        h: Harness::new(),
        buf: vec![0.0f32; 2 * 441],
        allocs: 0,
        real_frames: 0,
        starved_callbacks: 0,
    };
    rig.h
        .engine
        .load(vec![track(1, &long), track(2, &a)], 0, 0)
        .expect("load");
    rig.settle();
    rig.run(20, true);
    // Queue edit while the resampler is running: the tail is replaced (and the edit applied, not
    // refused as stale) by the three tracks that change the rate at every boundary.
    let edit = rig
        .h
        .engine
        .replace_after(
            QueueItemId(1),
            vec![track(3, &a), track(4, &b), track(5, &c), track(6, &a)],
        )
        .expect("edit");
    rig.h.expect_edit_applied(edit);
    rig.run(20, true);
    // Device change inside the resampled track.
    rig.h
        .engine
        .switch_device(Some("other".into()))
        .expect("switch");
    rig.run(20, false);
    rig.settle();
    rig.run(20, true);
    // Seek inside the resampled track.
    rig.h.engine.seek(5_000).expect("seek");
    rig.run(20, false);
    rig.settle();
    rig.run(30, true);
    // Play on through 44.1 -> 48 -> 44.1 -> 96 -> 48 kHz (about 199,000 frames from here), then
    // past the end of the queue.
    rig.run(440, true);
    rig.run(100, false);

    assert_eq!(
        rig.allocs, 0,
        "the render callback allocated or freed {} times",
        rig.allocs
    );
    assert!(
        rig.real_frames > 190_000,
        "test must cross the rate changes, rendered {} frames",
        rig.real_frames
    );
    assert!(
        rig.starved_callbacks > 0,
        "test must also cover callbacks with nothing to play"
    );
}
