//! Tests of the public surface: the provider views, the thread loop on a fake OS, and the
//! non-Windows `Unsupported` path. The default test run never touches the real audio stack.

use std::sync::Mutex;
use std::time::Instant;

use tinwave_core::{SystemClock, SystemRng};
use tinwave_signals::{
    BusConfig, OutputClass, OutputTransport, SignalDrain, SignalKind, SignalPayload,
    SignalSettings, Volume,
};

use super::backend::RawEvent;
use super::fake::{
    bluetooth_headset, device_id, speakers, FakeBackend, FakeHandle, FakePlayback, VecSink,
};
use super::*;
use crate::provider::{OutputDeviceFields, SystemVolumeFields};

fn real_clock_bus() -> (SignalBus, SignalDrain) {
    SignalBus::channel(
        BusConfig::new(device_id()),
        SystemClock,
        Arc::new(SignalSettings::new()),
    )
    .unwrap()
}

fn config(bus: SignalBus, playback: Arc<dyn PlaybackContextSource>) -> AudioMonitorConfig {
    AudioMonitorConfig {
        bus,
        secret: DeviceSecret::generate(&mut SystemRng).unwrap(),
        playback,
        target: OutputTarget::SystemDefault,
    }
}

/// Starts the real thread loop on a fake OS. `sender_out` receives the callback sender, so the
/// test can play the part of the OS callbacks.
fn start_fake(
    config: AudioMonitorConfig,
    os: &FakeHandle,
    sender_out: &Arc<Mutex<Option<runtime::EventSender>>>,
) -> AudioMonitor {
    let cache = Arc::new(ArcSwap::from_pointee(ContextFields::default()));
    let stats = Arc::new(StatsCells::default());
    let (os, sender_out) = (os.clone(), sender_out.clone());
    let thread = runtime::spawn(config, cache.clone(), stats.clone(), move |ctx| {
        *sender_out.lock().unwrap() = Some(ctx.events);
        Ok(os.backend())
    })
    .unwrap();
    AudioMonitor {
        cache,
        stats,
        thread: Some(thread),
    }
}

fn output_fields(monitor: &AudioMonitor) -> Option<OutputDeviceFields> {
    let mut fields = ContextFields::default();
    monitor.output_device().snapshot(&mut fields);
    fields.output_device
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_thread_loop_follows_events_flushes_gestures_and_shuts_down() {
    let os = FakeHandle::default();
    os.add("spk", speakers());
    os.add("bt", bluetooth_headset());
    os.os().default_id = Some("spk".into());
    let (bus, mut drain) = real_clock_bus();
    let sender = Arc::new(Mutex::new(None));
    let monitor = start_fake(config(bus.clone(), FakePlayback::playing()), &os, &sender);

    wait_until("the initial bind", || output_fields(&monitor).is_some());
    assert_eq!(
        output_fields(&monitor).map(|f| (f.class, f.transport)),
        Some((OutputClass::Speakers, OutputTransport::Hda))
    );

    // A drag, 30 % -> 33 %. The thread waits out the 400 ms window by itself.
    let send = sender.lock().unwrap().clone().unwrap();
    let generation = os.generation();
    for scalar in [0.31, 0.32, 0.33] {
        send.send(RawEvent::Volume {
            at: bus.now(),
            generation,
            context: 0,
            scalar,
            muted: false,
        });
    }
    let mut sink = VecSink::default();
    wait_until("the coalesced SystemVolumeChanged", || {
        sink.pump_all(&mut drain)
            .iter()
            .any(|e| e.kind() == SignalKind::SystemVolumeChanged)
    });
    let volumes: Vec<_> = sink
        .events
        .iter()
        .filter_map(|e| match e.payload {
            SignalPayload::SystemVolumeChanged(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(volumes.len(), 1, "one event for the whole drag");
    assert_eq!(
        (volumes[0].from, volumes[0].to),
        (
            Volume::from_linear(0.30).unwrap(),
            Volume::from_linear(0.33).unwrap()
        )
    );
    assert_eq!(monitor.stats().volume_events, 3);

    // The listener picks the headset in Tinwave: a command reaches the thread and re-binds.
    monitor
        .set_target(OutputTarget::Device("bt".into()))
        .unwrap();
    wait_until("the re-bind", || {
        output_fields(&monitor).is_some_and(|f| f.class == OutputClass::Bluetooth)
    });
    let mut volume = ContextFields::default();
    monitor.system_volume().snapshot(&mut volume);
    assert!(volume.system_volume.is_some_and(|v| v.gain_uncertain));

    drop(monitor);
    assert_eq!(
        os.os().bound_id,
        None,
        "dropping the monitor unregisters from the OS"
    );
}

#[test]
fn a_backend_that_cannot_start_degrades_to_unknown_fields() {
    let (bus, _drain) = real_clock_bus();
    let cache = Arc::new(ArcSwap::from_pointee(ContextFields::default()));
    let stats = Arc::new(StatsCells::default());
    let thread = runtime::spawn::<FakeBackend, _>(
        config(bus, FakePlayback::playing()),
        cache.clone(),
        stats.clone(),
        |_| Err("no audio service".to_owned()),
    )
    .unwrap();
    wait_until("the degraded flag", || stats.snapshot().degraded);
    assert_eq!(**cache.load(), ContextFields::default());
    drop(thread); // must not hang
}

/// Off Windows: starts, reports `Unsupported`, every field is unknown, no thread.
#[cfg(not(windows))]
#[test]
fn unsupported_off_windows() {
    let (bus, _drain) = real_clock_bus();
    let monitor = AudioMonitor::start(config(bus, FakePlayback::playing())).unwrap();
    for capability in [
        monitor.output_device().capability(),
        monitor.system_volume().capability(),
        AudioMonitor::capability(),
    ] {
        assert!(matches!(capability, Capability::Unsupported(reason) if !reason.is_empty()));
    }
    let mut fields = ContextFields::default();
    monitor.output_device().snapshot(&mut fields);
    monitor.system_volume().snapshot(&mut fields);
    assert_eq!(fields, ContextFields::default());
    assert!(matches!(
        monitor.set_target(OutputTarget::SystemDefault),
        Err(ContextError::ProviderStopped)
    ));
}

#[cfg(windows)]
#[test]
fn supported_on_windows_with_the_documented_tiers() {
    // No monitor is started: the default test run does not touch the real audio stack.
    assert_eq!(AudioMonitor::capability(), Capability::Supported);
    let cache = Arc::new(ArcSwap::from_pointee(ContextFields::default()));
    let output = OutputDeviceProvider {
        cache: cache.clone(),
    };
    let volume = SystemVolumeProvider { cache };
    assert_eq!(
        (output.tier(), output.id()),
        (Tier::T1, ProviderId::OutputDevice)
    );
    assert_eq!(
        (volume.tier(), volume.id()),
        (Tier::T0, ProviderId::SystemVolume)
    );
    assert_eq!(output.capability(), Capability::Supported);
    assert_eq!(volume.capability(), Capability::Supported);
}

#[test]
fn snapshot_only_touches_its_own_field() {
    let cache = Arc::new(ArcSwap::from_pointee(ContextFields {
        output_device: Some(OutputDeviceFields {
            class: OutputClass::Speakers,
            transport: OutputTransport::Hda,
            form_factor: Some(1),
        }),
        system_volume: Some(SystemVolumeFields {
            volume: Volume::MAX,
            muted: true,
            endpoint_gain_db: None,
            gain_uncertain: false,
        }),
    }));
    let output = OutputDeviceProvider {
        cache: cache.clone(),
    };
    let volume = SystemVolumeProvider { cache };
    let mut fields = ContextFields::default();
    output.snapshot(&mut fields);
    assert!(fields.output_device.is_some() && fields.system_volume.is_none());
    volume.snapshot(&mut fields);
    assert!(fields.output_device.is_some() && fields.system_volume.is_some());
    assert_eq!(output.max_age(), Duration::MAX);
}

#[test]
fn the_event_context_guid_is_a_fixed_v4_uuid() {
    // Version nibble 4, variant 10xx: not GUID_NULL, so "no context" is never mistaken for us.
    assert_ne!(EVENT_CONTEXT_GUID, 0);
    assert_eq!((EVENT_CONTEXT_GUID >> 76) & 0xf, 4);
    assert_eq!((EVENT_CONTEXT_GUID >> 62) & 0x3, 2);
}

/// A fake OS whose first `bind` returns, then waits for the test: while the provider thread is
/// held there, the test can overflow the event queue deterministically.
struct GatedBackend {
    inner: FakeBackend,
    gate: Option<crossbeam_channel::Receiver<()>>,
}

impl backend::Backend for GatedBackend {
    fn bind(
        &mut self,
        target: backend::BindTarget<'_>,
        generation: u64,
    ) -> Result<backend::BoundEndpoint, backend::BindError> {
        let bound = self.inner.bind(target, generation);
        if let Some(gate) = self.gate.take() {
            gate.recv().unwrap();
        }
        bound
    }

    fn unbind(&mut self) {
        self.inner.unbind();
    }

    fn gain_db(&mut self) -> Option<f32> {
        self.inner.gain_db()
    }
}

/// Holds the thread after its first bind, sends `events` into a queue of capacity 1, runs
/// `change_os` while it is held, then releases it. Returns the monitor and the bus drain.
fn overflow_one_slot_queue(
    events: Vec<RawEvent>,
    change_os: impl FnOnce(&FakeHandle),
) -> (AudioMonitor, FakeHandle, runtime::EventSender) {
    let os = FakeHandle::default();
    os.add("spk", speakers());
    os.add("bt", bluetooth_headset());
    os.os().default_id = Some("spk".into());
    let (bus, _drain) = real_clock_bus();
    let cache = Arc::new(ArcSwap::from_pointee(ContextFields::default()));
    let stats = Arc::new(StatsCells::default());
    let (gate_tx, gate_rx) = crossbeam_channel::bounded(1);
    let (sender_tx, sender_rx) = crossbeam_channel::bounded(1);
    let factory_os = os.clone();
    let thread = runtime::spawn_with_queue(
        config(bus, FakePlayback::playing()),
        cache.clone(),
        stats.clone(),
        1,
        move |ctx| {
            sender_tx.send(ctx.events).unwrap();
            Ok(GatedBackend {
                inner: factory_os.backend(),
                gate: Some(gate_rx),
            })
        },
    )
    .unwrap();
    let sender = sender_rx.recv().unwrap();
    // The thread is now inside its first bind (or about to be): wait until it has bound.
    wait_until("the first bind", || os.os().bound_id.is_some());
    for event in events {
        sender.send(event);
    }
    change_os(&os);
    gate_tx.send(()).unwrap();
    let monitor = AudioMonitor {
        cache,
        stats,
        thread: Some(thread),
    };
    // The caller keeps the sender alive, as the real backend does for its callbacks.
    (monitor, os, sender)
}

fn default_change(role: backend::Role, id: &str) -> RawEvent {
    RawEvent::DefaultChanged {
        at: tinwave_core::UtcMillis::new(0),
        flow: backend::Flow::Render,
        role,
        id: Some(id.to_owned()),
    }
}

#[test]
fn a_lost_default_change_makes_the_thread_re_query_the_os() {
    // Slot 1 holds a harmless repeat; the multimedia change that matters is dropped.
    let (monitor, _os, _sender) = overflow_one_slot_queue(
        vec![
            default_change(backend::Role::Console, "bt"),
            default_change(backend::Role::Multimedia, "bt"),
        ],
        |os| os.os().default_id = Some("bt".into()),
    );
    wait_until("the resync to bind the new default", || {
        output_fields(&monitor).is_some_and(|f| f.class == OutputClass::Bluetooth)
    });
    let stats = monitor.stats();
    assert_eq!((stats.callback_dropped, stats.resyncs), (1, 1));
}

#[test]
fn a_lost_volume_step_does_not_trigger_a_resync() {
    let volume = |scalar| RawEvent::Volume {
        at: tinwave_core::UtcMillis::new(0),
        generation: 0,
        context: 0,
        scalar,
        muted: false,
    };
    let (monitor, _os, _sender) = overflow_one_slot_queue(vec![volume(0.4), volume(0.5)], |_| {});
    wait_until("the queued event to be handled", || {
        monitor.stats().stale_events >= 1
    });
    let stats = monitor.stats();
    assert_eq!((stats.callback_dropped, stats.resyncs), (1, 0));
}
