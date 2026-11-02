//! The provider logic: a state machine over [`RawEvent`]s. No OS calls except through [`Backend`].
//!
//! # What it does
//!
//! - Tracks the bound endpoint's `(volume, muted)` and publishes them (and the device class) to
//!   the `ArcSwap` cache that `ContextProvider::snapshot` reads. Tracking never stops, so
//!   `from` is right even when nothing plays.
//! - Drops changes made with Tinwave's own event-context GUID.
//! - Diffs one notification into `MuteToggled` (mute changed) and a `SystemVolumeChanged` tick
//!   (scalar changed, quantized to 1 %), both when both changed. The tick goes through a
//!   [`GestureCoalescer`] (400 ms trailing window).
//! - Re-binds: default-device change (acted on once, for `(render, multimedia)`; the other two
//!   roles and repeats of the bound id are ignored), bound-device unplug or removal, and
//!   recovery when a device becomes active again. A failed bind leaves every field unknown.
//! - Emits `OutputDeviceChanged` when the bound endpoint changes. The initial bind is not a
//!   change and emits nothing.
//!
//! # The playback gate
//!
//! A `SessionId` is a column of every event, and the "while Tinwave plays" gate belongs to the
//! consumer, so the engine asks a [`PlaybackContextSource`] for the current session. `None`
//! means nothing is playing: the change is tracked and cached but no signal is emitted (counted
//! in [`AudioStats::suppressed_not_playing`](super::AudioStats)).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use tinwave_core::UtcMillis;
use tinwave_signals::{
    GestureCoalescer, GestureTick, GestureTime, Hash32, InputSource, MutePayload, OutputClass,
    OutputDevicePayload, OutputTransport, SignalBus, SignalEvent, SignalKind, SignalPayload,
    SystemVolumePayload, Volume,
};
use tracing::{debug, trace, warn};

use super::backend::{
    Backend, BindError, BindTarget, BoundEndpoint, DeviceState, Flow, RawEvent, Role,
    EVENT_CONTEXT_GUID,
};
use super::classify::{self, Transport};
use super::identity::DeviceSecret;
use super::stats::StatsCells;
use super::{OutputTarget, PlaybackContextSource};
use crate::provider::{ContextFields, OutputDeviceFields, SystemVolumeFields};

/// The endpoint the volume notification is bound to, and what is known about it.
#[derive(Debug, Clone)]
struct Binding {
    id: String,
    generation: u64,
    class: OutputClass,
    transport: Transport,
    form_factor: Option<u8>,
    device_hash: Hash32,
    /// The slider scalar in whole percent.
    percent: u8,
    muted: bool,
    gain_db: Option<f32>,
}

impl Binding {
    fn fields(&self) -> ContextFields {
        let volume = volume_of_percent(self.percent);
        ContextFields {
            output_device: Some(OutputDeviceFields {
                class: self.class,
                transport: self.transport.signal(),
                form_factor: self.form_factor,
            }),
            system_volume: volume.map(|volume| SystemVolumeFields {
                volume,
                muted: self.muted,
                endpoint_gain_db: self.gain_db,
                gain_uncertain: matches!(self.class, OutputClass::Hdmi | OutputClass::Bluetooth),
            }),
        }
    }
}

/// The slider scalar (0..=1) as whole percent; `None` for NaN or infinity.
fn percent_of_scalar(scalar: f32) -> Option<u8> {
    scalar
        .is_finite()
        // In 0..=100 after the clamp, so the cast cannot truncate.
        .then(|| (scalar.clamp(0.0, 1.0) * 100.0).round() as u8)
}

fn volume_of_percent(percent: u8) -> Option<Volume> {
    match Volume::from_linear(f32::from(percent) / 100.0) {
        Ok(v) => Some(v),
        Err(error) => {
            trace!(%error, "volume out of range");
            None
        }
    }
}

/// Quantizes a dB reading to 0.25 dB, so a hardware step does not look like a new value.
fn quantize_gain_db(db: f32) -> Option<f32> {
    db.is_finite().then(|| (db * 4.0).round() / 4.0)
}

/// The provider state machine. Owned and driven by the provider thread.
pub(crate) struct Engine<B: Backend> {
    backend: B,
    bus: SignalBus,
    coalescer: GestureCoalescer,
    playback: Arc<dyn PlaybackContextSource>,
    secret: DeviceSecret,
    target: OutputTarget,
    bound: Option<Binding>,
    generation: u64,
    gain_dirty: bool,
    cache: Arc<ArcSwap<ContextFields>>,
    stats: Arc<StatsCells>,
}

impl<B: Backend> std::fmt::Debug for Engine<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("target", &self.target)
            .field("bound", &self.bound.as_ref().map(|b| b.class))
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl<B: Backend> Engine<B> {
    pub(crate) fn new(
        backend: B,
        bus: SignalBus,
        playback: Arc<dyn PlaybackContextSource>,
        secret: DeviceSecret,
        target: OutputTarget,
        cache: Arc<ArcSwap<ContextFields>>,
        stats: Arc<StatsCells>,
    ) -> Self {
        Self {
            backend,
            coalescer: GestureCoalescer::new(bus.clone()),
            bus,
            playback,
            secret,
            target,
            bound: None,
            generation: 0,
            gain_dirty: false,
            cache,
            stats,
        }
    }

    /// The initial bind. It is not a change: nothing is emitted, the cache is just filled.
    pub(crate) fn start(&mut self) {
        let bound = self.bind_current_target();
        self.bound = bound;
        self.refresh_gain();
        self.publish();
    }

    /// Handles one OS notification.
    pub(crate) fn handle(&mut self, event: RawEvent) {
        match event {
            RawEvent::Volume {
                at,
                generation,
                context,
                scalar,
                muted,
            } => self.on_volume(at, generation, context, scalar, muted),
            RawEvent::DefaultChanged { flow, role, id, .. } => {
                self.on_default_changed(flow, role, id)
            }
            RawEvent::StateChanged { id, state, .. } => self.on_state_changed(&id, state),
            RawEvent::Removed { id, .. } => self.on_state_changed(&id, DeviceState::NotPresent),
        }
    }

    /// Call after a batch of events, before waiting: refreshes the gain reading once per batch
    /// (not per step of a 60-event burst) and ends gestures whose window has passed.
    pub(crate) fn settle(&mut self) {
        if std::mem::take(&mut self.gain_dirty) {
            self.refresh_gain();
            self.publish();
        }
        let _ = self.coalescer.flush_due();
    }

    /// How long the thread may sleep before a gesture must be flushed; `None` when idle.
    pub(crate) fn wait_hint(&self) -> Option<Duration> {
        let deadline = self.coalescer.next_deadline()?;
        let ms = deadline.as_i64().saturating_sub(self.bus.now().as_i64());
        // At least 1 ms so a deadline that is "now" does not make the thread spin.
        Some(Duration::from_millis(u64::try_from(ms).unwrap_or(0).max(1)))
    }

    /// Changes which endpoint is followed.
    pub(crate) fn set_target(&mut self, target: OutputTarget) {
        if self.target == target {
            return;
        }
        self.target = target;
        let next = self.rebind_to_target();
        self.transition(next);
    }

    /// Re-queries the OS after device events were lost (the runtime's queue overflowed): binds
    /// to what the target asks for now. Emits `OutputDeviceChanged` only if the endpoint
    /// differs from the bound one.
    pub(crate) fn resync(&mut self) {
        StatsCells::bump(&self.stats.resyncs);
        let next = self.rebind_to_target();
        self.transition(next);
    }

    /// Ends gestures and removes the OS registrations.
    pub(crate) fn shutdown(&mut self) {
        let _ = self.coalescer.flush_all();
        self.backend.unbind();
    }

    // ---- volume ----

    fn on_volume(
        &mut self,
        at: UtcMillis,
        generation: u64,
        context: u128,
        scalar: f32,
        muted: bool,
    ) {
        let Some(percent) = percent_of_scalar(scalar) else {
            warn!("ignoring a non-finite volume notification");
            return;
        };
        let (old_percent, old_muted, class) = match self.bound.as_mut() {
            Some(b) if b.generation == generation => {
                let old = (b.percent, b.muted, b.class);
                b.percent = percent;
                b.muted = muted;
                old
            }
            _ => {
                StatsCells::bump(&self.stats.stale_events);
                return;
            }
        };

        StatsCells::bump(&self.stats.volume_events);
        let lag = self.bus.now().as_i64().saturating_sub(at.as_i64());
        self.stats
            .max_lag_ms
            .fetch_max(u64::try_from(lag).unwrap_or(0), Ordering::Relaxed);
        self.gain_dirty = true;
        self.publish();

        if context == EVENT_CONTEXT_GUID {
            StatsCells::bump(&self.stats.self_changes);
            return;
        }
        if muted != old_muted {
            self.emit_mute(muted);
        }
        if percent != old_percent {
            self.tick_system_volume(old_percent, percent, class);
        }
    }

    fn emit_mute(&mut self, muted: bool) {
        if !self.bus.enabled(SignalKind::MuteToggled) {
            return;
        }
        let Some(playing) = self.playback.current() else {
            StatsCells::bump(&self.stats.suppressed_not_playing);
            return;
        };
        let payload = SignalPayload::MuteToggled(MutePayload {
            muted,
            input: InputSource::System,
        });
        self.emit(SignalKind::MuteToggled, payload, playing.event);
    }

    fn tick_system_volume(&mut self, from: u8, to: u8, endpoint_class: OutputClass) {
        if !self.bus.enabled(SignalKind::SystemVolumeChanged) {
            return;
        }
        let (Some(from), Some(to)) = (volume_of_percent(from), volume_of_percent(to)) else {
            return;
        };
        let Some(playing) = self.playback.current() else {
            StatsCells::bump(&self.stats.suppressed_not_playing);
            return;
        };
        let _ = self.coalescer.system_volume(|| GestureTick {
            context: playing.event,
            payload: SystemVolumePayload {
                from,
                to,
                gesture: GestureTime::from_ms(0),
                input: InputSource::System,
                section: playing.section,
                since_track_start: playing.since_track_start,
                since_section_start: playing.since_section_start,
                endpoint_class,
            },
        });
    }

    fn emit(
        &self,
        kind: SignalKind,
        payload: SignalPayload,
        context: tinwave_signals::EventContext,
    ) {
        let result = self.bus.emit_with(kind, |stamp| SignalEvent {
            id: stamp.id,
            ts: stamp.ts,
            session: context.session,
            recording: context.recording,
            source: context.source,
            pos_ds: context.pos_ds,
            payload,
            ctx: context.ctx,
        });
        if let Err(error) = result {
            warn!(%error, %kind, "signal rejected by the bus");
        }
    }

    // ---- devices ----

    fn on_default_changed(&mut self, flow: Flow, role: Role, id: Option<String>) {
        // Windows fires once per role; Tinwave opens its stream with the multimedia role.
        if flow != Flow::Render
            || role != Role::Multimedia
            || self.target != OutputTarget::SystemDefault
        {
            StatsCells::bump(&self.stats.default_repeats);
            return;
        }
        let bound_id = self.bound.as_ref().map(|b| b.id.as_str());
        if id.as_deref() == bound_id && bound_id.is_some() {
            StatsCells::bump(&self.stats.default_repeats);
            return;
        }
        let next = match id {
            Some(id) => self.rebind(BindTarget::Device(&id)),
            None => self.rebind_nothing(),
        };
        self.transition(next);
    }

    /// A device became active, or stopped being. Losing the bound endpoint re-queries the
    /// default straight away.
    ///
    /// The order of Windows' notifications is not guaranteed. If the default has already moved
    /// when the state change arrives (`A` unplugged, default now `B`), this is one
    /// `OutputDeviceChanged` for `A -> B` and the later default-change callback is a repeat.
    /// If the state change arrives **first** (the default still names `A`, which is no longer
    /// active), nothing can be bound: `A -> Unknown` is emitted, then the default-change
    /// callback binds `B` and `Unknown -> B` is emitted. Two events is legal (ADR 0004
    /// section 5); the order seen on real hardware is PO-RUNBOOK R6/R7.
    fn on_state_changed(&mut self, id: &str, state: DeviceState) {
        let is_bound = self.bound.as_ref().is_some_and(|b| b.id == id);
        if is_bound && state != DeviceState::Active {
            // Unplugged or removed. Windows moves the default on its own, but the order of
            // the notifications is not guaranteed, so ask for the default right away.
            let next = self.rebind_to_target();
            self.transition(next);
            return;
        }
        if !is_bound && state == DeviceState::Active && self.bound.is_none() {
            let wanted = match &self.target {
                OutputTarget::SystemDefault => true,
                OutputTarget::Device(chosen) => chosen == id,
            };
            if wanted {
                let next = self.rebind_to_target();
                self.transition(next);
            }
        }
    }

    /// Drops the current registration and binds to what the target asks for.
    fn rebind_to_target(&mut self) -> Option<Binding> {
        let target = self.target.clone();
        match &target {
            OutputTarget::SystemDefault => self.rebind(BindTarget::SystemDefault),
            OutputTarget::Device(id) => self.rebind(BindTarget::Device(id)),
        }
    }

    fn rebind(&mut self, target: BindTarget<'_>) -> Option<Binding> {
        // A gesture on the old endpoint ends there: from/to across devices mean nothing.
        let _ = self.coalescer.flush_all();
        self.backend.unbind();
        let next = self.bind(target);
        if next.is_none() {
            StatsCells::bump(&self.stats.bind_failures);
        }
        next
    }

    fn rebind_nothing(&mut self) -> Option<Binding> {
        let _ = self.coalescer.flush_all();
        self.backend.unbind();
        None
    }

    fn bind_current_target(&mut self) -> Option<Binding> {
        let target = self.target.clone();
        match &target {
            OutputTarget::SystemDefault => self.bind(BindTarget::SystemDefault),
            OutputTarget::Device(id) => self.bind(BindTarget::Device(id)),
        }
    }

    /// Binds and builds the tracked state. A failure is logged and means "unknown".
    fn bind(&mut self, target: BindTarget<'_>) -> Option<Binding> {
        // Bumped even when binding fails, so a late callback of a failed attempt is stale.
        self.generation += 1;
        let generation = self.generation;
        match self.backend.bind(target, generation) {
            Ok(endpoint) => Some(self.describe(endpoint, generation)),
            Err(error @ (BindError::NoDevice | BindError::NotActive)) => {
                debug!(%error, "nothing to bind the system-volume notification to");
                None
            }
            Err(error) => {
                warn!(%error, "could not bind the system-volume notification");
                None
            }
        }
    }

    fn describe(&self, endpoint: BoundEndpoint, generation: u64) -> Binding {
        let BoundEndpoint {
            id,
            description,
            volume_scalar,
            muted,
        } = endpoint;
        let internal = description
            .jack
            .is_some_and(classify::JackDescription::is_internal);
        let class = classify::classify(description.form_factor, description.transport, internal);
        let device_hash = self.secret.device_hash(&id);
        Binding {
            id,
            generation,
            class,
            transport: description.transport,
            form_factor: description
                .form_factor
                .map(|f| classify::form_factor_byte(Some(f))),
            device_hash,
            percent: percent_of_scalar(volume_scalar).unwrap_or(0),
            muted,
            gain_db: None,
        }
    }

    /// Installs `next` as the bound endpoint; emits `OutputDeviceChanged` if the endpoint differs.
    fn transition(&mut self, next: Option<Binding>) {
        let changed = self.bound.as_ref().map(|b| &b.id) != next.as_ref().map(|b| &b.id);
        self.bound = next;
        self.gain_dirty = false;
        self.refresh_gain();
        self.publish();
        if changed {
            StatsCells::bump(&self.stats.rebinds);
            self.emit_device_changed();
        }
    }

    fn emit_device_changed(&mut self) {
        if !self.bus.enabled(SignalKind::OutputDeviceChanged) {
            return;
        }
        let Some(playing) = self.playback.current() else {
            StatsCells::bump(&self.stats.suppressed_not_playing);
            return;
        };
        let payload = match &self.bound {
            Some(b) => OutputDevicePayload {
                class: b.class,
                transport: b.transport.signal(),
                form_factor: classify::form_factor_byte(b.form_factor.map(u32::from)),
                device_hash: b.device_hash,
            },
            None => OutputDevicePayload {
                class: OutputClass::Unknown,
                transport: OutputTransport::Other,
                form_factor: classify::form_factor_byte(None),
                device_hash: Hash32(0),
            },
        };
        self.emit(
            SignalKind::OutputDeviceChanged,
            SignalPayload::OutputDeviceChanged(payload),
            playing.event,
        );
    }

    fn refresh_gain(&mut self) {
        if self.bound.is_none() {
            return;
        }
        let gain = self.backend.gain_db().and_then(quantize_gain_db);
        if let Some(b) = self.bound.as_mut() {
            b.gain_db = gain;
        }
    }

    fn publish(&self) {
        let fields = self.bound.as_ref().map(Binding::fields).unwrap_or_default();
        self.cache.store(Arc::new(fields));
    }

    #[cfg(test)]
    pub(crate) fn coalescer_idle(&self) -> bool {
        self.coalescer.is_idle()
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
