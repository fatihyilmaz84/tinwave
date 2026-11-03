//! What exclusive-mode stream reopens mean to the signals (T-0057): a reopen at another sample
//! rate is not a change of mode and says nothing; losing exclusive mode because the device refuses
//! is one `ExclusiveModeToggled { on: false }`, once.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use common::write_sweep_piece;
use controller_support::{entry, kinds, only, Rig};
use tinwave_playback::ManualOutput;
use tinwave_signals::{InputSource, PlayOrigin, QueueOrigin, SignalKind, SignalPayload};

/// Half a second at the 48 kHz timeline.
const FRAMES: usize = 24_000;

fn rig(tag: &str) -> Rig {
    let (output, handle) = ManualOutput::new();
    handle.set_exclusive(true);
    Rig::with_output(tag, Box::new(output), handle)
}

/// Three half-second files at 44.1, 96 and 44.1 kHz, queued in that order.
fn play_three(rig: &mut Rig, between: impl FnOnce(&mut Rig)) -> Vec<tinwave_signals::SignalEvent> {
    let mut entries = Vec::new();
    for (id, rate) in [(1u64, 44_100u32), (2, 96_000), (3, 44_100)] {
        let path = rig.dir.path(&format!("t{id}.wav"));
        write_sweep_piece(&path, rate, 0, rate as usize / 2, 2);
        entries.push(entry(id, &path, FRAMES, QueueOrigin::Manual));
    }
    rig.select(entries, 0, PlayOrigin::Album, InputSource::Mouse);
    rig.run_until("the first track was heard", |r| {
        !r.of_kind(SignalKind::PlayStarted).is_empty()
    });
    between(rig);
    rig.play_to_end();
    rig.signals()
}

#[test]
fn reopening_at_the_file_rate_is_not_a_change_of_exclusive_mode() {
    let mut rig = rig("excl-sig-reopen");
    let signals = play_three(&mut rig, |_| {});
    // Three tracks played through three streams; the mode never changed.
    let opens = rig.out.opens();
    assert_eq!(
        opens.iter().map(|o| o.1).collect::<Vec<_>>(),
        [48_000, 44_100, 96_000, 44_100]
    );
    assert_eq!(
        signals
            .iter()
            .filter(|e| e.kind() == SignalKind::ExclusiveModeToggled)
            .count(),
        0,
        "{:?}",
        kinds(&signals)
    );
    assert_eq!(
        signals
            .iter()
            .filter(|e| e.kind() == SignalKind::Completed)
            .count(),
        3
    );
}

#[test]
fn losing_exclusive_mode_to_a_refusing_device_is_one_toggle_and_the_queue_plays_on() {
    let mut rig = rig("excl-sig-refused");
    let out = rig.out.clone();
    // After the first track started, the device stops giving out exclusive streams.
    let signals = play_three(&mut rig, |_| out.set_device_rates(&[]));
    let toggle = only(&signals, SignalKind::ExclusiveModeToggled);
    let SignalPayload::ExclusiveModeToggled(p) = toggle.payload else {
        panic!("kind");
    };
    assert!(!p.on, "exclusive mode ended");
    // 96 kHz was asked for once, then the engine's own rate, and the third track did not ask.
    assert_eq!(
        rig.out.opens().iter().map(|o| o.1).collect::<Vec<_>>(),
        [48_000, 44_100, 96_000, 48_000]
    );
    // All three tracks completed: the refusal cost the sample-rate fidelity, not the listening.
    assert_eq!(
        signals
            .iter()
            .filter(|e| e.kind() == SignalKind::Completed)
            .count(),
        3,
        "{:?}",
        kinds(&signals)
    );
}
