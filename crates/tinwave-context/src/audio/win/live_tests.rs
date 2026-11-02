//! Live tests against the real Windows audio stack.
//!
//! **These change the real system volume and mute** of the default render device by a few
//! percent for a few seconds, and restore both (also when an assertion fails: the restore is a
//! `Drop` guard, and the workspace does not use `panic = "abort"`). They are `#[ignore]`d, so
//! `cargo test` never runs them. Run them on purpose, with the speakers low and nothing else
//! playing:
//!
//! ```text
//! cargo test -p tinwave-context live_ -- --ignored --test-threads=1
//! ```
//!
//! A process that is killed (Ctrl+C, a task kill) cannot restore anything. If that ever happens,
//! set the volume and unmute from the Windows volume flyout.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tinwave_core::{SystemClock, SystemRng};
use tinwave_signals::{BusConfig, SignalBus, SignalDrain, SignalPayload, SignalSettings, Volume};

use super::com::LiveEndpoint;
use crate::audio::backend::EVENT_CONTEXT_GUID;
use crate::audio::fake::{device_id, FakePlayback, VecSink};
use crate::audio::{AudioMonitor, AudioMonitorConfig, DeviceSecret, OutputTarget};

/// What the shell or the mixer would pass: not ours, not `GUID_NULL`.
const FOREIGN_CONTEXT: u128 = 0x0bad_c0de_0000_4000_8000_0000_0000_f00d;

/// Puts the volume and mute back, whatever happens.
struct Restore<'a> {
    endpoint: &'a LiveEndpoint,
    scalar: f32,
    muted: bool,
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        // Own context: the restore itself is not a signal.
        let _ = self.endpoint.set_mute(self.muted, EVENT_CONTEXT_GUID);
        let _ = self.endpoint.set_scalar(self.scalar, EVENT_CONTEXT_GUID);
        eprintln!(
            "live test: after the restore volume {:.2} (original {:.2}), muted {} (original {})",
            self.endpoint.scalar(),
            self.scalar,
            self.endpoint.muted(),
            self.muted
        );
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn percent(scalar: f32) -> u8 {
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

fn volume_percent(percent: u8) -> Volume {
    Volume::from_linear(f32::from(percent) / 100.0).unwrap()
}

fn collect(sink: &mut VecSink, drain: &mut SignalDrain) -> (usize, usize) {
    let events = sink.pump_all(drain);
    let volumes = events
        .iter()
        .filter(|e| matches!(e.payload, SignalPayload::SystemVolumeChanged(_)))
        .count();
    let mutes = events
        .iter()
        .filter(|e| matches!(e.payload, SignalPayload::MuteToggled(_)))
        .count();
    (volumes, mutes)
}

#[test]
#[ignore = "changes the real system volume and mute (restores them); run with --ignored --test-threads=1"]
fn live_own_changes_are_dropped_and_external_changes_are_signals() {
    let endpoint = LiveEndpoint::default_render().expect("a default render device is required");
    eprintln!(
        "live test: volume {:.2}, muted {} before the test",
        endpoint.scalar(),
        endpoint.muted()
    );
    let _restore = Restore {
        endpoint: &endpoint,
        scalar: endpoint.scalar(),
        muted: endpoint.muted(),
    };

    let (bus, mut drain) = SignalBus::channel(
        BusConfig::new(device_id()),
        SystemClock,
        Arc::new(SignalSettings::new()),
    )
    .unwrap();
    let monitor = AudioMonitor::start(AudioMonitorConfig {
        bus,
        secret: DeviceSecret::generate(&mut SystemRng).unwrap(),
        playback: FakePlayback::playing(),
        target: OutputTarget::SystemDefault,
    })
    .unwrap();
    let mut sink = VecSink::default();
    let mut fields = crate::provider::ContextFields::default();
    wait_until("the provider to bind the default render device", || {
        crate::provider::ContextProvider::snapshot(&monitor.system_volume(), &mut fields);
        fields.system_volume.is_some()
    });

    // A few percent away from where the user left it.
    let start = endpoint.scalar();
    let step = if start > 0.5 { -0.04 } else { 0.04 };

    // 1. Our own context (unmuted first, so the mute step below is a clean toggle).
    endpoint.set_mute(false, EVENT_CONTEXT_GUID).unwrap();
    endpoint
        .set_scalar(start + step, EVENT_CONTEXT_GUID)
        .unwrap();
    wait_until("the own-context notifications", || {
        monitor.stats().self_changes >= 1
    });
    std::thread::sleep(Duration::from_millis(700)); // longer than the 400 ms window
    assert_eq!(
        collect(&mut sink, &mut drain),
        (0, 0),
        "changes made with our own event context must not be signals"
    );

    // 2. A foreign context and the null context, two quick steps: one gesture, one event.
    let before = percent(endpoint.scalar());
    endpoint
        .set_scalar(start + 2.0 * step, FOREIGN_CONTEXT)
        .unwrap();
    endpoint.set_scalar(start + 3.0 * step, 0).unwrap();
    let after = percent(endpoint.scalar());
    assert_ne!(before, after, "the endpoint must have moved");
    wait_until("the SystemVolumeChanged", || {
        collect(&mut sink, &mut drain).0 >= 1
    });
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        collect(&mut sink, &mut drain).0,
        1,
        "one event for the gesture"
    );
    let payload = sink
        .events
        .iter()
        .find_map(|e| match e.payload {
            SignalPayload::SystemVolumeChanged(p) => Some(p),
            _ => None,
        })
        .unwrap();
    assert_eq!(payload.from, volume_percent(before));
    assert_eq!(payload.to, volume_percent(after));

    // 3. Mute is its own signal and does not move the scalar.
    endpoint.set_mute(true, FOREIGN_CONTEXT).unwrap();
    wait_until("the MuteToggled", || collect(&mut sink, &mut drain).1 >= 1);
    endpoint.set_mute(false, 0).unwrap();
    wait_until("the second MuteToggled", || {
        collect(&mut sink, &mut drain).1 >= 2
    });
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(collect(&mut sink, &mut drain), (1, 2));
}
