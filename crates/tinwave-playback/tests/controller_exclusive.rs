//! What exclusive-mode stream reopens mean to the signals (T-0057): a reopen at another sample
//! rate is not a change of mode and says nothing; losing exclusive mode because the device refuses
//! says nothing either (the device decided, the listener did not express a preference); the
//! listener's own change of the setting is one `ExclusiveModeToggled` each way.
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
fn losing_exclusive_mode_to_a_refusing_device_is_no_preference_signal_and_the_queue_plays_on() {
    let mut rig = rig("excl-sig-refused");
    let out = rig.out.clone();
    // After the first track started, the device stops giving out exclusive streams.
    let signals = play_three(&mut rig, |_| out.set_device_rates(&[]));
    // The device decided, not the listener: nothing about exclusive mode is signalled.
    assert_eq!(
        signals
            .iter()
            .filter(|e| e.kind() == SignalKind::ExclusiveModeToggled)
            .count(),
        0,
        "{:?}",
        kinds(&signals)
    );
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

#[test]
fn the_listener_turning_exclusive_mode_off_and_on_is_one_toggle_each() {
    let mut rig = rig("excl-sig-listener");
    let path = rig.dir.path("t.wav");
    write_sweep_piece(&path, 44_100, 0, 44_100, 2);
    let e = entry(1, &path, 2 * FRAMES, QueueOrigin::Manual);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.run_until("the track was heard", |r| {
        !r.of_kind(SignalKind::PlayStarted).is_empty()
    });
    let _ = rig.new_signals();

    // The setting changes and the stream is opened again: a stream that is what was asked for.
    rig.out.set_exclusive(false);
    rig.ctl
        .switch_device(Some("speakers".into()))
        .expect("switch");
    let SignalPayload::ExclusiveModeToggled(p) =
        only(&rig.new_signals(), SignalKind::ExclusiveModeToggled).payload
    else {
        panic!("kind");
    };
    assert!(!p.on);

    rig.out.set_exclusive(true);
    rig.ctl.switch_device(Some("dac".into())).expect("switch");
    let SignalPayload::ExclusiveModeToggled(p) =
        only(&rig.new_signals(), SignalKind::ExclusiveModeToggled).payload
    else {
        panic!("kind");
    };
    assert!(p.on);
}

/// A one-second 44.1 kHz track: the engine's rate is 48 kHz, so exclusive mode asks for a reopen.
fn one_track(rig: &Rig) -> tinwave_playback::QueueEntry {
    let path = rig.dir.path("t.wav");
    write_sweep_piece(&path, 44_100, 0, 44_100, 2);
    entry(1, &path, 2 * FRAMES, QueueOrigin::Manual)
}

fn toggled(events: &[tinwave_signals::SignalEvent]) -> bool {
    let SignalPayload::ExclusiveModeToggled(p) =
        only(events, SignalKind::ExclusiveModeToggled).payload
    else {
        panic!("kind");
    };
    p.on
}

#[test]
fn turning_exclusive_off_and_on_while_the_device_has_us_in_shared_is_one_toggle_each() {
    let mut rig = rig("excl-sig-in-fallback");
    // The device accepts no exclusive rate: the 44.1 kHz track is played from a shared stream.
    rig.out.set_device_rates(&[]);
    let e = one_track(&rig);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.run_until("the track was heard", |r| {
        !r.of_kind(SignalKind::PlayStarted).is_empty()
    });
    assert_eq!(
        rig.out.opens().iter().map(|o| o.1).collect::<Vec<_>>(),
        [48_000, 44_100, 48_000],
        "the exclusive stream was refused and the engine fell back"
    );
    assert!(
        !kinds(&rig.new_signals()).contains(&SignalKind::ExclusiveModeToggled),
        "the fallback itself is no preference"
    );

    // The stream is shared already; the listener turns the setting off: a preference.
    rig.out.set_exclusive(false);
    rig.ctl.poll();
    assert!(!toggled(&rig.new_signals()));
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty(), "one reading, one signal");

    // And back on: the device still refuses, the stream stays shared, the signal is the setting's.
    rig.out.set_exclusive(true);
    rig.ctl.poll();
    assert!(toggled(&rig.new_signals()));
    rig.ctl.switch_device(Some("dac".into())).expect("switch");
    rig.ctl.poll();
    assert!(
        rig.new_signals().is_empty(),
        "a refused reopen with the setting unchanged says nothing"
    );
}

#[test]
fn a_toggle_made_before_a_session_survives_a_fallback_reopen() {
    // T-0057 review: the setting was turned on outside a session and the stream reopened
    // exclusive; then the first load reopened the output at a rate the device refused. The
    // fallback must not erase the listener's toggle.
    let (output, handle) = ManualOutput::new();
    let mut rig = Rig::with_output("excl-sig-deferred-fallback", Box::new(output), handle);
    rig.out.set_exclusive(true);
    rig.ctl.switch_device(None).expect("switch");
    assert!(rig.signals().is_empty(), "no session yet");
    rig.out.set_device_rates(&[]);

    let e = one_track(&rig);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.run_until("the track was heard", |r| {
        !r.of_kind(SignalKind::PlayStarted).is_empty()
    });
    assert_eq!(
        rig.out.opens().iter().map(|o| o.1).collect::<Vec<_>>(),
        [48_000, 48_000, 44_100, 48_000],
        "the exclusive stream was refused and the engine fell back"
    );
    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::ExclusiveModeToggled,
            SignalKind::PlayStarted
        ]
    );
    assert!(toggled(&signals));
}
