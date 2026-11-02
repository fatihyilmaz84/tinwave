//! The render callback still performs zero heap allocations when the engine is driven through the
//! controller (commands, polls, signal emission and gesture flushes all run on the control
//! thread, none of it may reach the callback). The test thread plays the audio callback, so the
//! counted region is exactly each render call.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use controller_support::*;
use tinwave_playback::LoadIntent;
use tinwave_signals::{InputSource, PlayOrigin, QueueOrigin, SignalKind};
use tinwave_test_alloc::{count_allocations, CountingAlloc};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

const SECOND: usize = 48_000;

fn callbacks(rig: &mut Rig, count: usize, allocs: &mut u64, real: &mut usize) {
    let mut buf = vec![0.0f32; 2 * 441];
    for _ in 0..count {
        let (frames, a) = count_allocations(|| rig.out.render(&mut buf, 480));
        *allocs += a;
        *real += frames.expect("stream open");
        rig.ctl.poll();
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
}

#[test]
fn the_callback_allocates_nothing_while_the_controller_emits_signals() {
    let mut rig = Rig::new("ctl-rt-alloc");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Album),
        entry(2, &b, 10 * SECOND, QueueOrigin::Album),
    ];
    let (mut allocs, mut real) = (0u64, 0usize);

    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    callbacks(&mut rig, 40, &mut allocs, &mut real);
    rig.settle();
    callbacks(&mut rig, 60, &mut allocs, &mut real);
    for step in 0..8_u8 {
        rig.ctl
            .set_volume(1.0 - f32::from(step) * 0.1, InputSource::Keyboard)
            .expect("volume");
        callbacks(&mut rig, 4, &mut allocs, &mut real);
    }
    rig.ctl.pause(InputSource::MediaKey);
    callbacks(&mut rig, 6, &mut allocs, &mut real);
    rig.ctl.resume(InputSource::MediaKey);
    callbacks(&mut rig, 6, &mut allocs, &mut real);
    rig.ctl.seek_ms(5_000, InputSource::Mouse).expect("seek");
    callbacks(&mut rig, 20, &mut allocs, &mut real);
    rig.settle();
    rig.clock.advance(3_000);
    callbacks(&mut rig, 20, &mut allocs, &mut real);
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::MediaKey)
        .expect("next");
    callbacks(&mut rig, 20, &mut allocs, &mut real);
    rig.settle();
    callbacks(&mut rig, 60, &mut allocs, &mut real);

    assert_eq!(
        allocs, 0,
        "the render callback allocated or freed {allocs} times"
    );
    assert!(
        real > 20_000,
        "the test must render audio, got {real} frames"
    );
    let kinds = kinds(&rig.signals());
    for expected in [
        SignalKind::PlayStarted,
        SignalKind::VolumeChanged,
        SignalKind::Paused,
        SignalKind::Resumed,
        SignalKind::SeekForward,
        SignalKind::Skipped,
    ] {
        assert!(
            kinds.contains(&expected),
            "{expected} was not emitted: {kinds:?}"
        );
    }
}
