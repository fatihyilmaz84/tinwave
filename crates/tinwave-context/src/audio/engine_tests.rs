//! Engine tests: fake OS events, a `ManualClock`, an in-memory sink. No real audio stack.

use std::sync::atomic::Ordering;

use tinwave_core::{ManualClock, UtcMillis};
use tinwave_signals::{
    BusConfig, GestureTime, InputSource, MutePayload, OutputDevicePayload, SignalCategory,
    SignalDrain, SignalEvent, SignalPayload, SignalSettings, SystemVolumePayload,
};

use super::*;
use crate::audio::fake::{
    bluetooth_headset, device_id, monitor, speakers, FakeBackend, FakeHandle, FakePlayback,
    VecSink, T0,
};

/// A guid other than ours and not `GUID_NULL` (what the shell or the mixer would pass).
const FOREIGN_CONTEXT: u128 = 0x1234_5678_9abc_def0_1234_5678_9abc_def0;

struct Rig {
    engine: Engine<FakeBackend>,
    os: FakeHandle,
    clock: ManualClock,
    bus: SignalBus,
    drain: SignalDrain,
    sink: VecSink,
    taken: usize,
    playback: Arc<FakePlayback>,
    cache: Arc<ArcSwap<ContextFields>>,
    stats: Arc<StatsCells>,
    secret: DeviceSecret,
}

impl Rig {
    /// `spk` (Realtek line-out, 30 %), `bt` (headset, 50 %) and `hdmi` (monitor, 80 %);
    /// the default and the bound endpoint is `spk`.
    fn new() -> Self {
        Self::with(OutputTarget::SystemDefault, Some("spk"))
    }

    fn with(target: OutputTarget, default_id: Option<&str>) -> Self {
        let os = FakeHandle::default();
        os.add("spk", speakers());
        os.add("bt", bluetooth_headset());
        os.add("hdmi", monitor());
        os.os().default_id = default_id.map(str::to_owned);
        Self::over(os, target)
    }

    fn over(os: FakeHandle, target: OutputTarget) -> Self {
        let clock = ManualClock::new(UtcMillis::new(T0));
        let (bus, drain) = SignalBus::channel(
            BusConfig::new(device_id()),
            clock.clone(),
            Arc::new(SignalSettings::new()),
        )
        .unwrap();
        let playback = FakePlayback::playing();
        let cache = Arc::new(ArcSwap::from_pointee(ContextFields::default()));
        let stats = Arc::new(StatsCells::default());
        let secret = DeviceSecret::from_bytes([9; super::super::identity::DEVICE_SECRET_LEN]);
        let mut engine = Engine::new(
            os.backend(),
            bus.clone(),
            playback.clone(),
            secret.clone(),
            target,
            cache.clone(),
            stats.clone(),
        );
        engine.start();
        Self {
            engine,
            os,
            clock,
            bus,
            drain,
            sink: VecSink::default(),
            taken: 0,
            playback,
            cache,
            stats,
            secret,
        }
    }

    fn generation(&self) -> u64 {
        self.os.generation()
    }

    /// An OS volume notification with the given context, stamped now.
    fn notify(&mut self, scalar: f32, muted: bool, context: u128) {
        let generation = self.generation();
        self.notify_gen(generation, scalar, muted, context);
    }

    fn notify_gen(&mut self, generation: u64, scalar: f32, muted: bool, context: u128) {
        self.engine.handle(RawEvent::Volume {
            at: self.bus.now(),
            generation,
            context,
            scalar,
            muted,
        });
        self.engine.settle();
    }

    /// An external change (null context, as the shell reports it).
    fn external(&mut self, scalar: f32, muted: bool) {
        self.notify(scalar, muted, 0);
    }

    fn default_changed(&mut self, flow: Flow, role: Role, id: Option<&str>) {
        self.engine.handle(RawEvent::DefaultChanged {
            at: self.bus.now(),
            flow,
            role,
            id: id.map(str::to_owned),
        });
        self.engine.settle();
    }

    fn state_changed(&mut self, id: &str, state: DeviceState) {
        self.engine.handle(RawEvent::StateChanged {
            at: self.bus.now(),
            id: id.to_owned(),
            state,
        });
        self.engine.settle();
    }

    /// Moves time and lets the engine end gestures that are due.
    fn advance(&mut self, ms: i64) {
        self.clock.advance(ms);
        self.engine.settle();
    }

    /// Events emitted since the last call.
    fn events(&mut self) -> Vec<SignalEvent> {
        let all = self.sink.pump_all(&mut self.drain);
        let new = all[self.taken..].to_vec();
        self.taken = all.len();
        new
    }

    /// Lets every gesture window pass, then returns the new events.
    fn settled_events(&mut self) -> Vec<SignalEvent> {
        self.advance(1_000);
        self.events()
    }

    fn fields(&self) -> ContextFields {
        **self.cache.load()
    }
}

fn volumes(events: &[SignalEvent]) -> Vec<SystemVolumePayload> {
    events
        .iter()
        .filter_map(|e| match e.payload {
            SignalPayload::SystemVolumeChanged(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn mutes(events: &[SignalEvent]) -> Vec<MutePayload> {
    events
        .iter()
        .filter_map(|e| match e.payload {
            SignalPayload::MuteToggled(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn devices(events: &[SignalEvent]) -> Vec<OutputDevicePayload> {
    events
        .iter()
        .filter_map(|e| match e.payload {
            SignalPayload::OutputDeviceChanged(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn vol(percent: u8) -> Volume {
    Volume::from_linear(f32::from(percent) / 100.0).unwrap()
}

// ---- start-up and the cache ----

#[test]
fn the_initial_bind_fills_the_cache_and_emits_nothing() {
    let mut r = Rig::new();
    assert_eq!(r.events(), []);
    assert_eq!(r.settled_events(), []);
    let fields = r.fields();
    assert_eq!(
        fields.output_device,
        Some(OutputDeviceFields {
            class: OutputClass::Speakers,
            transport: OutputTransport::Hda,
            form_factor: Some(1),
        })
    );
    // -40.1 dB is quantized to 0.25 dB.
    assert_eq!(
        fields.system_volume,
        Some(SystemVolumeFields {
            volume: vol(30),
            muted: false,
            endpoint_gain_db: Some(-40.0),
            gain_uncertain: false,
        })
    );
}

#[test]
fn bluetooth_and_hdmi_gain_is_marked_uncertain() {
    for (default, class) in [("bt", OutputClass::Bluetooth), ("hdmi", OutputClass::Hdmi)] {
        let r = Rig::with(OutputTarget::SystemDefault, Some(default));
        let fields = r.fields();
        assert_eq!(fields.output_device.map(|f| f.class), Some(class));
        assert!(fields.system_volume.is_some_and(|v| v.gain_uncertain));
    }
}

#[test]
fn no_render_device_means_unknown_fields_and_no_error() {
    let r = Rig::with(OutputTarget::SystemDefault, None);
    assert_eq!(r.fields(), ContextFields::default());
}

// ---- coalescing ----

#[test]
fn a_60_step_burst_in_200_ms_is_one_event_with_the_first_from_and_last_to() {
    let mut r = Rig::new();
    r.advance(10_000);
    let started = r.bus.now();
    for step in 0..60u8 {
        // 31 % ... 90 %, one hardware step of 1 % every 3 ms: 177 ms from first to last.
        r.external(f32::from(31 + step) / 100.0, false);
        r.advance(3);
    }
    let last_tick = UtcMillis::new(started.as_i64() + 59 * 3);
    assert_eq!(r.events(), [], "nothing while the gesture is in progress");

    // 399 ms after the last step the gesture is still open ...
    r.advance(399 - 3);
    assert_eq!(r.events(), []);
    // ... and at 400 ms it ends.
    r.advance(1);
    let events = r.events();
    assert_eq!(events.len(), 1, "60 steps, one event: {events:?}");
    let [event] = volumes(&events)[..] else {
        panic!("expected one SystemVolumeChanged, got {events:?}");
    };
    assert_eq!(event.from, vol(30));
    assert_eq!(event.to, vol(90));
    assert_eq!(event.gesture, GestureTime::from_ms(177));
    assert_eq!(event.input, InputSource::System);
    assert_eq!(event.endpoint_class, OutputClass::Speakers);
    assert_eq!(events[0].ts, last_tick, "the gesture ends at its last step");
    assert_eq!(events[0].session, FakePlayback::session());
    assert_eq!(r.stats.snapshot().volume_events, 60);
}

#[test]
fn two_gestures_more_than_400_ms_apart_are_two_events() {
    let mut r = Rig::new();
    r.external(0.40, false);
    r.advance(500);
    r.external(0.20, false);
    let events = r.settled_events();
    let v = volumes(&events);
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].from, v[0].to), (vol(30), vol(40)));
    assert_eq!((v[1].from, v[1].to), (vol(40), vol(20)));
}

#[test]
fn a_gesture_put_back_where_it_started_is_no_event() {
    let mut r = Rig::new();
    r.external(0.40, false);
    r.advance(100);
    r.external(0.30, false);
    assert_eq!(r.settled_events(), []);
}

#[test]
fn the_engine_tells_the_thread_how_long_it_may_sleep() {
    let mut r = Rig::new();
    assert_eq!(r.engine.wait_hint(), None, "idle: sleep until an event");
    r.external(0.40, false);
    assert_eq!(r.engine.wait_hint(), Some(Duration::from_millis(400)));
    r.advance(150);
    assert_eq!(r.engine.wait_hint(), Some(Duration::from_millis(250)));
    r.advance(250);
    assert_eq!(r.engine.wait_hint(), None);
    assert!(r.engine.coalescer_idle());
}

// ---- one notification, two signals ----

#[test]
fn a_mute_only_change_is_a_mute_toggle_and_nothing_else() {
    let mut r = Rig::new();
    r.external(0.30, true);
    let events = r.settled_events();
    assert_eq!(
        mutes(&events),
        [MutePayload {
            muted: true,
            input: InputSource::System
        }]
    );
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(r.fields().system_volume.is_some_and(|v| v.muted));

    r.external(0.30, false);
    let events = r.settled_events();
    assert_eq!(mutes(&events).first().map(|m| m.muted), Some(false));
    assert_eq!(events.len(), 1);
}

#[test]
fn a_scalar_only_change_is_a_system_volume_change_and_nothing_else() {
    let mut r = Rig::new();
    r.external(0.45, false);
    let events = r.settled_events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(volumes(&events).len(), 1);
}

#[test]
fn a_notification_that_changes_both_emits_both() {
    let mut r = Rig::new();
    r.external(0.10, true);
    let events = r.settled_events();
    assert_eq!(mutes(&events).len(), 1);
    assert_eq!(volumes(&events).len(), 1);
    assert_eq!(events.len(), 2);
}

#[test]
fn a_change_below_one_percent_is_not_a_change() {
    let mut r = Rig::new();
    // 0.3004 and 0.2996 both quantize to 30 %.
    r.external(0.3004, false);
    r.external(0.2996, false);
    assert_eq!(r.settled_events(), []);
    assert_eq!(r.stats.snapshot().volume_events, 2);
}

#[test]
fn a_non_finite_scalar_is_ignored() {
    let mut r = Rig::new();
    r.external(f32::NAN, false);
    r.external(f32::INFINITY, false);
    assert_eq!(r.settled_events(), []);
    assert_eq!(r.stats.snapshot().volume_events, 0);
}

#[test]
fn the_endpoint_gain_is_read_once_per_batch_and_quantized() {
    let mut r = Rig::new();
    r.os.os().devices.get_mut("spk").unwrap().gain_db = Some(-30.13);
    r.external(0.40, false);
    assert_eq!(
        r.fields().system_volume.and_then(|v| v.endpoint_gain_db),
        Some(-30.25)
    );
}

// ---- own changes ----

#[test]
fn changes_with_tinwaves_own_context_guid_emit_nothing_but_are_tracked() {
    let mut r = Rig::new();
    r.notify(0.50, false, EVENT_CONTEXT_GUID);
    r.notify(0.50, true, EVENT_CONTEXT_GUID);
    assert_eq!(r.settled_events(), [], "own changes are not the listener's");
    let stats = r.stats.snapshot();
    assert_eq!((stats.self_changes, stats.volume_events), (2, 2));
    let tracked = r.fields().system_volume.unwrap();
    assert_eq!((tracked.volume, tracked.muted), (vol(50), true));

    // The listener's next change starts from what Tinwave set, so `from` is right.
    r.notify(0.60, true, FOREIGN_CONTEXT);
    let events = r.settled_events();
    let v = volumes(&events);
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].from, v[0].to), (vol(50), vol(60)));
    assert_eq!(mutes(&events), [], "mute was already on");
}

#[test]
fn a_foreign_guid_and_the_null_guid_are_both_external() {
    for context in [0, FOREIGN_CONTEXT] {
        let mut r = Rig::new();
        r.notify(0.55, false, context);
        assert_eq!(
            volumes(&r.settled_events()).len(),
            1,
            "context {context:#x}"
        );
    }
}

// ---- default device ----

fn permutations() -> [[Role; 3]; 6] {
    use Role::{Communications as C, Console as K, Multimedia as M};
    [
        [K, M, C],
        [K, C, M],
        [M, K, C],
        [M, C, K],
        [C, K, M],
        [C, M, K],
    ]
}

#[test]
fn three_default_callbacks_for_one_switch_are_one_output_device_changed() {
    for order in permutations() {
        let mut r = Rig::new();
        r.os.os().default_id = Some("bt".into());
        for role in order {
            r.default_changed(Flow::Render, role, Some("bt"));
        }
        let events = r.settled_events();
        let changes = devices(&events);
        assert_eq!(changes.len(), 1, "order {order:?}: {events:?}");
        assert_eq!(
            changes[0],
            OutputDevicePayload {
                class: OutputClass::Bluetooth,
                transport: OutputTransport::Bluetooth,
                form_factor: 3,
                device_hash: r.secret.device_hash("bt"),
            }
        );
        assert_eq!(events.len(), 1);
        let binds =
            r.os.os()
                .calls
                .iter()
                .filter(|c| c.starts_with("bind"))
                .count();
        assert_eq!(
            binds, 2,
            "the start-up bind and one re-bind, order {order:?}"
        );
        assert_eq!(
            r.fields().output_device.map(|f| f.class),
            Some(OutputClass::Bluetooth)
        );
    }
}

#[test]
fn repeated_multimedia_callbacks_for_the_bound_endpoint_are_ignored() {
    let mut r = Rig::new();
    r.default_changed(Flow::Render, Role::Multimedia, Some("spk"));
    r.default_changed(Flow::Render, Role::Multimedia, Some("spk"));
    assert_eq!(r.settled_events(), []);
    assert_eq!(r.stats.snapshot().default_repeats, 2);
}

#[test]
fn capture_defaults_are_not_our_output() {
    let mut r = Rig::new();
    r.default_changed(Flow::Capture, Role::Multimedia, Some("bt"));
    r.default_changed(Flow::Other, Role::Multimedia, Some("bt"));
    assert_eq!(r.settled_events(), []);
    assert_eq!(
        r.fields().output_device.map(|f| f.class),
        Some(OutputClass::Speakers)
    );
}

#[test]
fn the_volume_follows_the_new_default_and_the_old_endpoint_is_stale() {
    let mut r = Rig::new();
    let old_generation = r.generation();
    r.os.os().default_id = Some("bt".into());
    r.default_changed(Flow::Render, Role::Multimedia, Some("bt"));
    assert_eq!(r.os.os().bound_id.as_deref(), Some("bt"));
    assert_ne!(r.generation(), old_generation);
    let _ = r.events();

    // A late notification from the registration we removed.
    r.notify_gen(old_generation, 0.95, false, 0);
    assert_eq!(r.settled_events(), []);
    assert_eq!(r.stats.snapshot().stale_events, 1);
    assert_eq!(r.fields().system_volume.map(|v| v.volume), Some(vol(50)));

    // The new endpoint's own notifications work, and carry its class.
    r.external(0.60, false);
    let v = volumes(&r.settled_events());
    assert_eq!(v.len(), 1);
    assert_eq!(
        (v[0].from, v[0].to, v[0].endpoint_class),
        (vol(50), vol(60), OutputClass::Bluetooth)
    );
}

#[test]
fn a_gesture_in_progress_ends_on_the_old_endpoint_when_the_default_changes() {
    let mut r = Rig::new();
    r.external(0.40, false);
    r.advance(100);
    r.os.os().default_id = Some("bt".into());
    r.default_changed(Flow::Render, Role::Multimedia, Some("bt"));
    let events = r.events();
    assert_eq!(events.len(), 2, "{events:?}");
    let v = volumes(&events);
    assert_eq!(
        (v[0].from, v[0].to, v[0].endpoint_class),
        (vol(30), vol(40), OutputClass::Speakers)
    );
    assert!(
        matches!(events[0].payload, SignalPayload::SystemVolumeChanged(_)),
        "the volume gesture first, then the device change"
    );
    assert_eq!(devices(&events).len(), 1);
}

#[test]
fn no_default_device_any_more_is_unknown() {
    let mut r = Rig::new();
    r.os.os().default_id = None;
    r.default_changed(Flow::Render, Role::Multimedia, None);
    let events = r.settled_events();
    assert_eq!(
        devices(&events),
        [OutputDevicePayload {
            class: OutputClass::Unknown,
            transport: OutputTransport::Other,
            form_factor: 10,
            device_hash: Hash32(0),
        }]
    );
    assert_eq!(r.fields(), ContextFields::default());
    assert_eq!(r.os.os().bound_id, None);
    // A volume notification from the endpoint that went away is stale.
    r.external(0.9, false);
    assert_eq!(r.settled_events(), []);
}

// ---- unplug, removal, bind failure ----

#[test]
fn unplugging_the_bound_device_falls_back_to_the_new_default_in_one_event() {
    let mut r = Rig::with(OutputTarget::SystemDefault, Some("bt"));
    r.os.os().devices.get_mut("bt").unwrap().state = DeviceState::Unplugged;
    r.os.os().default_id = Some("spk".into());
    r.state_changed("bt", DeviceState::Unplugged);
    // Windows' own default-change notification arrives afterwards: a repeat.
    r.default_changed(Flow::Render, Role::Multimedia, Some("spk"));
    r.default_changed(Flow::Render, Role::Console, Some("spk"));
    let events = r.settled_events();
    assert_eq!(devices(&events).len(), 1, "{events:?}");
    assert_eq!(devices(&events)[0].class, OutputClass::Speakers);
    assert_eq!(
        r.fields().output_device.map(|f| f.class),
        Some(OutputClass::Speakers)
    );
}

#[test]
fn removing_the_only_device_goes_unknown_and_its_return_binds_again() {
    let mut r = Rig::new();
    r.os.os().devices.get_mut("spk").unwrap().state = DeviceState::NotPresent;
    r.os.os().default_id = None;
    r.engine.handle(RawEvent::Removed {
        at: r.bus.now(),
        id: "spk".into(),
    });
    r.engine.settle();
    let events = r.settled_events();
    assert_eq!(devices(&events).len(), 1);
    assert_eq!(devices(&events)[0].class, OutputClass::Unknown);
    assert_eq!(r.fields(), ContextFields::default());

    // The jack is plugged in again: the device is active and the default.
    r.os.os().devices.get_mut("spk").unwrap().state = DeviceState::Active;
    r.os.os().default_id = Some("spk".into());
    r.state_changed("spk", DeviceState::Active);
    let events = r.settled_events();
    assert_eq!(devices(&events).len(), 1, "{events:?}");
    assert_eq!(devices(&events)[0].class, OutputClass::Speakers);
    assert!(r.fields().output_device.is_some());
}

#[test]
fn state_changes_of_other_devices_are_ignored() {
    let mut r = Rig::new();
    r.state_changed("bt", DeviceState::Unplugged);
    r.state_changed("hdmi", DeviceState::Active);
    r.state_changed("bt", DeviceState::Active);
    assert_eq!(r.settled_events(), []);
    assert_eq!(r.os.os().bound_id.as_deref(), Some("spk"));
}

#[test]
fn a_failed_bind_leaves_the_fields_unknown_and_is_counted() {
    let os = FakeHandle::default();
    os.add("spk", speakers());
    os.os().default_id = Some("spk".into());
    os.os().fail_next_bind = Some("access denied".into());
    let mut r = Rig::over(os, OutputTarget::SystemDefault);
    assert_eq!(r.fields(), ContextFields::default(), "start-up bind failed");
    assert_eq!(r.events(), [], "the first bind is not a change");

    // A default change later binds fine: Unknown -> Speakers is a change.
    r.default_changed(Flow::Render, Role::Multimedia, Some("spk"));
    let events = r.settled_events();
    assert_eq!(devices(&events).len(), 1);
    assert_eq!(devices(&events)[0].class, OutputClass::Speakers);

    // And a bind that fails mid-session means Unknown, with the failure counted.
    r.os.os().fail_next_bind = Some("device invalidated".into());
    r.os.os().devices.insert("bt".into(), bluetooth_headset());
    r.default_changed(Flow::Render, Role::Multimedia, Some("bt"));
    let events = r.settled_events();
    assert_eq!(devices(&events).len(), 1);
    assert_eq!(devices(&events)[0].class, OutputClass::Unknown);
    assert_eq!(r.fields(), ContextFields::default());
    assert_eq!(r.stats.snapshot().bind_failures, 1);
}

// ---- chosen device ----

#[test]
fn a_chosen_device_ignores_default_changes_and_has_no_fallback() {
    let mut r = Rig::with(OutputTarget::Device("bt".into()), Some("spk"));
    assert_eq!(
        r.fields().output_device.map(|f| f.class),
        Some(OutputClass::Bluetooth)
    );
    r.default_changed(Flow::Render, Role::Multimedia, Some("hdmi"));
    assert_eq!(r.settled_events(), []);

    // The headset goes away: Unknown, not "whatever is default".
    r.os.os().devices.get_mut("bt").unwrap().state = DeviceState::Unplugged;
    r.state_changed("bt", DeviceState::Unplugged);
    let events = r.settled_events();
    assert_eq!(devices(&events).len(), 1);
    assert_eq!(devices(&events)[0].class, OutputClass::Unknown);
    assert_eq!(r.fields(), ContextFields::default());

    // It comes back.
    r.os.os().devices.get_mut("bt").unwrap().state = DeviceState::Active;
    r.state_changed("bt", DeviceState::Active);
    assert_eq!(devices(&r.settled_events()).len(), 1);
    assert_eq!(
        r.fields().output_device.map(|f| f.class),
        Some(OutputClass::Bluetooth)
    );
}

#[test]
fn switching_the_target_rebinds_and_emits_once() {
    let mut r = Rig::new();
    r.engine.set_target(OutputTarget::Device("hdmi".into()));
    let events = r.settled_events();
    assert_eq!(devices(&events).len(), 1);
    assert_eq!(devices(&events)[0].class, OutputClass::Hdmi);
    // The same target again is a no-op.
    r.engine.set_target(OutputTarget::Device("hdmi".into()));
    assert_eq!(r.settled_events(), []);
    // Back to the default (speakers).
    r.engine.set_target(OutputTarget::SystemDefault);
    assert_eq!(devices(&r.settled_events())[0].class, OutputClass::Speakers);
    // Choosing the device that is already bound is a rebind but not a change.
    r.engine.set_target(OutputTarget::Device("spk".into()));
    assert_eq!(r.settled_events(), []);
}

// ---- the playback gate, toggles, shutdown ----

#[test]
fn nothing_is_emitted_while_nothing_plays_but_tracking_continues() {
    let mut r = Rig::new();
    r.playback.set_playing(false);
    r.external(0.40, false);
    r.external(0.40, true);
    r.default_changed(Flow::Render, Role::Multimedia, Some("bt"));
    assert_eq!(r.settled_events(), []);
    assert_eq!(r.stats.snapshot().suppressed_not_playing, 3);
    assert_eq!(
        r.fields().output_device.map(|f| f.class),
        Some(OutputClass::Bluetooth)
    );

    // Playback starts: `from` is what the listener left it at, not what it was at start-up.
    r.playback.set_playing(true);
    r.external(0.55, true);
    let v = volumes(&r.settled_events());
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].from, v[0].to), (vol(50), vol(55)));
}

#[test]
fn a_disabled_category_builds_nothing_but_the_cache_keeps_current() {
    let mut r = Rig::new();
    r.bus.settings().set_category(SignalCategory::Volume, false);
    r.bus
        .settings()
        .set_category(SignalCategory::OutputSound, false);
    r.external(0.40, true);
    r.default_changed(Flow::Render, Role::Multimedia, Some("bt"));
    assert_eq!(r.settled_events(), []);
    assert_eq!(r.stats.snapshot().suppressed_not_playing, 0);
    assert_eq!(
        r.fields().output_device.map(|f| f.class),
        Some(OutputClass::Bluetooth)
    );
}

#[test]
fn shutdown_ends_the_gesture_and_removes_the_registration() {
    let mut r = Rig::new();
    r.external(0.40, false);
    r.engine.shutdown();
    assert_eq!(r.os.os().bound_id, None);
    let v = volumes(&r.events());
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].from, v[0].to), (vol(30), vol(40)));
}

#[test]
fn callback_lag_is_measured() {
    let mut r = Rig::new();
    let generation = r.generation();
    let at = UtcMillis::new(r.bus.now().as_i64() - 42);
    r.engine.handle(RawEvent::Volume {
        at,
        generation,
        context: 0,
        scalar: 0.4,
        muted: false,
    });
    assert_eq!(r.stats.max_lag_ms.load(Ordering::Relaxed), 42);
}
