//! Gesture coalescing: one event per gesture, not one per slider tick.
//!
//! `docs/rules/storage-budget.md` section 2 and `docs/rules/qml-ui.md`: volume and speed sliders,
//! the OS volume callbacks and repeated seeks are bursts. [`GestureCoalescer`] folds a burst into
//! one event: the **first** `from`, the **last** `to`.
//!
//! # Windows (one table)
//!
//! A gesture is a debounce: it ends after a window of silence, measured from the last tick.
//!
//! | Kind | Window | Why |
//! |---|---|---|
//! | `VolumeChanged` | 600 ms | design-system.md section 5: a wheel burst ends after 600 ms |
//! | `RateChanged` | 600 ms | same gesture shape as volume |
//! | `SystemVolumeChanged` | 400 ms | ADR 0004: the OS sends one callback per step; 400 ms trailing |
//! | `SeekForward` / `SeekBack` | 2 000 ms | storage-budget.md: seeks within 2 s merge |
//!
//! [`coalesce_window_ms`] is the table; the tests check every kind against it.
//!
//! # Driving it
//!
//! The coalescer owns no thread and no timer. The host calls [`GestureCoalescer::flush_due`]
//! when [`GestureCoalescer::next_deadline`] passes (a single-shot UI timer) or, simply, every
//! 100 ms, and [`GestureCoalescer::flush_all`] when the track changes, playback stops or the
//! app quits. Dropping the coalescer flushes whatever is pending. Ticks and flushes read the
//! bus's injected clock, so tests drive it with a `ManualClock`.
//!
//! # What a merged event contains
//!
//! - `from` (and `from_ds`, `section_from`) from the first tick; `to`, `section`, `input`, the
//!   `since_*` fields and the endpoint class from the last tick.
//! - `ts`, `pos_ds`, `source` and the context snapshot from the last tick (the catalog's
//!   trigger is "gesture ends"). A tick for another session or recording ends the gesture first.
//! - `gesture_ms` is the time from the start of the first tick (its own `gesture` duration
//!   before it arrived, 0 for an instant step) to the last tick.
//! - A gesture that ends where it began (`from == to`, e.g. the slider was dragged and put back)
//!   emits nothing: the listener changed nothing. Likewise a merged seek of 2 s or less
//!   (the catalog's threshold applies to the net seek). [`GestureCoalescer::suppressed`] counts
//!   these so they are not invisible.
//! - A merged seek is `SeekForward` or `SeekBack` by its **net** direction.

use tinwave_core::{RecordingId, SessionId, SourceId, UtcMillis};

use crate::bus::{EmitOutcome, SignalBus};
use crate::{
    ContextSnapshotId, GestureTime, RatePayload, SeekPayload, SignalKind, SignalPayload,
    SystemVolumePayload, VolumePayload,
};

/// Window of silence that ends an in-app `VolumeChanged` gesture.
pub const VOLUME_WINDOW_MS: u32 = 600;
/// Window of silence that ends a `RateChanged` gesture.
pub const RATE_WINDOW_MS: u32 = 600;
/// Trailing window that ends a `SystemVolumeChanged` gesture (ADR 0004).
pub const SYSTEM_VOLUME_WINDOW_MS: u32 = 400;
/// Window within which consecutive seeks merge.
pub const SEEK_WINDOW_MS: u32 = 2_000;
/// A merged seek is only an event when it moved the position by more than this many
/// deciseconds (2 s, the catalog's "seek of more than 2 s").
pub const SEEK_MIN_NET_DS: u32 = 20;

/// The coalescing window of a kind, `None` for kinds that are not coalesced.
#[must_use]
pub const fn coalesce_window_ms(kind: SignalKind) -> Option<u32> {
    match kind {
        SignalKind::VolumeChanged => Some(VOLUME_WINDOW_MS),
        SignalKind::RateChanged => Some(RATE_WINDOW_MS),
        SignalKind::SystemVolumeChanged => Some(SYSTEM_VOLUME_WINDOW_MS),
        SignalKind::SeekForward | SignalKind::SeekBack => Some(SEEK_WINDOW_MS),
        _ => None,
    }
}

/// The columns every event shares besides its id, time and payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventContext {
    /// Session the event belongs to.
    pub session: SessionId,
    /// Recording the event is about, if any.
    pub recording: Option<RecordingId>,
    /// Source of that recording, if any.
    pub source: Option<SourceId>,
    /// Playback position in deciseconds, if the event has one.
    pub pos_ds: Option<u32>,
    /// Context snapshot taken with the event, if context capture is on.
    pub ctx: Option<ContextSnapshotId>,
}

/// One tick of a gesture: where it happened and what it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GestureTick<P> {
    /// The event columns for the event this tick may become.
    pub context: EventContext,
    /// This tick's payload. `from`/`to` describe this tick's own step.
    pub payload: P,
}

/// What [`GestureCoalescer`] did with a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum TickOutcome {
    /// Held until the gesture ends (or merged into the gesture in progress).
    Buffered,
    /// The kind's category or tier is off: the constructor did not run.
    Disabled,
}

/// How a payload type merges across the ticks of one gesture.
trait Gesture: Copy {
    /// The kind whose toggle gates the gesture (both seek kinds share category and tier).
    const TOGGLE_KIND: SignalKind;
    const WINDOW_MS: u32;
    /// The duration this single tick spans (0 for an instant step).
    fn own_duration_ms(&self) -> u32;
    /// Folds a later tick into the gesture so far.
    fn absorb(&mut self, later: &Self);
    /// The finished event payload, `None` when the gesture changed nothing.
    fn finish(self, span_ms: u32) -> Option<SignalPayload>;
}

impl Gesture for VolumePayload {
    const TOGGLE_KIND: SignalKind = SignalKind::VolumeChanged;
    const WINDOW_MS: u32 = VOLUME_WINDOW_MS;

    fn own_duration_ms(&self) -> u32 {
        self.gesture.ms()
    }

    fn absorb(&mut self, later: &Self) {
        *self = Self {
            from: self.from,
            ..*later
        };
    }

    fn finish(self, span_ms: u32) -> Option<SignalPayload> {
        (self.from != self.to).then(|| {
            SignalPayload::VolumeChanged(Self {
                gesture: GestureTime::from_ms(span_ms),
                ..self
            })
        })
    }
}

impl Gesture for SystemVolumePayload {
    const TOGGLE_KIND: SignalKind = SignalKind::SystemVolumeChanged;
    const WINDOW_MS: u32 = SYSTEM_VOLUME_WINDOW_MS;

    fn own_duration_ms(&self) -> u32 {
        self.gesture.ms()
    }

    fn absorb(&mut self, later: &Self) {
        *self = Self {
            from: self.from,
            ..*later
        };
    }

    fn finish(self, span_ms: u32) -> Option<SignalPayload> {
        (self.from != self.to).then(|| {
            SignalPayload::SystemVolumeChanged(Self {
                gesture: GestureTime::from_ms(span_ms),
                ..self
            })
        })
    }
}

impl Gesture for RatePayload {
    const TOGGLE_KIND: SignalKind = SignalKind::RateChanged;
    const WINDOW_MS: u32 = RATE_WINDOW_MS;

    fn own_duration_ms(&self) -> u32 {
        0
    }

    fn absorb(&mut self, later: &Self) {
        *self = Self {
            from: self.from,
            ..*later
        };
    }

    fn finish(self, _span_ms: u32) -> Option<SignalPayload> {
        (self.from != self.to).then_some(SignalPayload::RateChanged(self))
    }
}

impl Gesture for SeekPayload {
    // `SeekForward` and `SeekBack` have the same category and tier, so one toggle covers both.
    const TOGGLE_KIND: SignalKind = SignalKind::SeekForward;
    const WINDOW_MS: u32 = SEEK_WINDOW_MS;

    fn own_duration_ms(&self) -> u32 {
        0
    }

    fn absorb(&mut self, later: &Self) {
        *self = Self {
            from_ds: self.from_ds,
            section_from: self.section_from,
            ..*later
        };
    }

    fn finish(self, _span_ms: u32) -> Option<SignalPayload> {
        let (from, to) = (self.from_ds.ds(), self.to_ds.ds());
        if from.abs_diff(to) <= SEEK_MIN_NET_DS {
            None
        } else if to > from {
            Some(SignalPayload::SeekForward(self))
        } else {
            Some(SignalPayload::SeekBack(self))
        }
    }
}

/// A gesture in progress.
#[derive(Debug, Clone, Copy)]
struct Pending<P> {
    /// When the gesture started: its first tick minus that tick's own duration.
    started: UtcMillis,
    /// When the last tick arrived.
    last: UtcMillis,
    context: EventContext,
    merged: P,
}

impl<P: Gesture> Pending<P> {
    fn is_due(&self, now: UtcMillis) -> bool {
        // A clock that moved backwards cannot measure silence: end the gesture rather than
        // hold it for an unbounded time.
        let silence = now.as_i64().saturating_sub(self.last.as_i64());
        silence < 0 || silence >= i64::from(P::WINDOW_MS)
    }

    fn deadline(&self) -> UtcMillis {
        UtcMillis::new(self.last.as_i64().saturating_add(i64::from(P::WINDOW_MS)))
    }
}

/// Folds gesture bursts into single events and emits them on the bus. See the module docs.
pub struct GestureCoalescer {
    bus: SignalBus,
    volume: Option<Pending<VolumePayload>>,
    system_volume: Option<Pending<SystemVolumePayload>>,
    rate: Option<Pending<RatePayload>>,
    seek: Option<Pending<SeekPayload>>,
    suppressed: u64,
}

impl std::fmt::Debug for GestureCoalescer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GestureCoalescer")
            .field("volume_pending", &self.volume.is_some())
            .field("system_volume_pending", &self.system_volume.is_some())
            .field("rate_pending", &self.rate.is_some())
            .field("seek_pending", &self.seek.is_some())
            .field("suppressed", &self.suppressed)
            .finish_non_exhaustive()
    }
}

impl GestureCoalescer {
    /// A coalescer emitting on `bus`, timed by the bus's clock.
    #[must_use]
    pub fn new(bus: SignalBus) -> Self {
        Self {
            bus,
            volume: None,
            system_volume: None,
            rate: None,
            seek: None,
            suppressed: 0,
        }
    }

    /// An in-app volume step. `build` is not called when the volume category is off.
    pub fn volume(&mut self, build: impl FnOnce() -> GestureTick<VolumePayload>) -> TickOutcome {
        tick(&self.bus, &mut self.volume, &mut self.suppressed, build)
    }

    /// An OS volume callback.
    pub fn system_volume(
        &mut self,
        build: impl FnOnce() -> GestureTick<SystemVolumePayload>,
    ) -> TickOutcome {
        tick(
            &self.bus,
            &mut self.system_volume,
            &mut self.suppressed,
            build,
        )
    }

    /// A playback-rate step.
    pub fn rate(&mut self, build: impl FnOnce() -> GestureTick<RatePayload>) -> TickOutcome {
        tick(&self.bus, &mut self.rate, &mut self.suppressed, build)
    }

    /// A user seek. Feed every seek, small ones included: the 2 s threshold applies to the
    /// merged result.
    pub fn seek(&mut self, build: impl FnOnce() -> GestureTick<SeekPayload>) -> TickOutcome {
        tick(&self.bus, &mut self.seek, &mut self.suppressed, build)
    }

    /// Emits every gesture whose window of silence has passed. Returns events handed to the
    /// bus (a disabled category or a full queue can make it fewer than gestures ended).
    pub fn flush_due(&mut self) -> usize {
        let now = self.bus.now();
        let (bus, suppressed) = (&self.bus, &mut self.suppressed);
        flush_if_due(bus, &mut self.volume, suppressed, now)
            + flush_if_due(bus, &mut self.system_volume, suppressed, now)
            + flush_if_due(bus, &mut self.rate, suppressed, now)
            + flush_if_due(bus, &mut self.seek, suppressed, now)
    }

    /// Ends every gesture now. Call when the track changes, playback stops or the app quits.
    pub fn flush_all(&mut self) -> usize {
        let (bus, suppressed) = (&self.bus, &mut self.suppressed);
        flush(bus, &mut self.volume, suppressed)
            + flush(bus, &mut self.system_volume, suppressed)
            + flush(bus, &mut self.rate, suppressed)
            + flush(bus, &mut self.seek, suppressed)
    }

    /// When the earliest gesture in progress ends if nothing else arrives, or `None` when idle.
    /// Schedule one timer for this instant.
    #[must_use]
    pub fn next_deadline(&self) -> Option<UtcMillis> {
        [
            self.volume.as_ref().map(Pending::deadline),
            self.system_volume.as_ref().map(Pending::deadline),
            self.rate.as_ref().map(Pending::deadline),
            self.seek.as_ref().map(Pending::deadline),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Whether any gesture is in progress.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.next_deadline().is_none()
    }

    /// Gestures that ended with no net change and so produced no event.
    #[must_use]
    pub fn suppressed(&self) -> u64 {
        self.suppressed
    }
}

impl Drop for GestureCoalescer {
    fn drop(&mut self) {
        let _ = self.flush_all();
    }
}

fn tick<P: Gesture>(
    bus: &SignalBus,
    slot: &mut Option<Pending<P>>,
    suppressed: &mut u64,
    build: impl FnOnce() -> GestureTick<P>,
) -> TickOutcome {
    if !bus.enabled(P::TOGGLE_KIND) {
        return TickOutcome::Disabled;
    }
    let now = bus.now();
    let GestureTick { context, payload } = build();
    // A tick joins the gesture in progress only if it is still inside the window and is about
    // the same session and recording.
    let continues = slot.as_ref().is_some_and(|p| {
        !p.is_due(now)
            && p.context.session == context.session
            && p.context.recording == context.recording
    });
    if !continues {
        let _ = flush(bus, slot, suppressed);
    }
    match slot {
        Some(pending) => {
            pending.merged.absorb(&payload);
            pending.context = context;
            pending.last = now;
        }
        None => {
            let own = i64::from(payload.own_duration_ms());
            *slot = Some(Pending {
                started: UtcMillis::new(now.as_i64().saturating_sub(own)),
                last: now,
                context,
                merged: payload,
            });
        }
    }
    TickOutcome::Buffered
}

fn flush_if_due<P: Gesture>(
    bus: &SignalBus,
    slot: &mut Option<Pending<P>>,
    suppressed: &mut u64,
    now: UtcMillis,
) -> usize {
    if slot.as_ref().is_some_and(|p| p.is_due(now)) {
        flush(bus, slot, suppressed)
    } else {
        0
    }
}

fn flush<P: Gesture>(
    bus: &SignalBus,
    slot: &mut Option<Pending<P>>,
    suppressed: &mut u64,
) -> usize {
    let Some(pending) = slot.take() else {
        return 0;
    };
    let span = pending
        .last
        .as_i64()
        .saturating_sub(pending.started.as_i64());
    let span_ms = u32::try_from(span.max(0)).unwrap_or(u32::MAX);
    let Some(payload) = pending.merged.finish(span_ms) else {
        *suppressed += 1;
        return 0;
    };
    match bus.emit_payload(payload, pending.last, pending.context) {
        EmitOutcome::Queued => 1,
        // Turned off since the first tick, or lost to overflow (counted by the bus).
        EmitOutcome::Disabled | EmitOutcome::Dropped => 0,
    }
}

#[cfg(test)]
mod tests {
    use tinwave_core::ManualClock;

    use super::*;
    use crate::test_support::{bus_with, context, recording, VecSink, T0};
    use crate::{
        ElapsedDs, InputSource, OutputClass, PositionDs, Rate, Section, SignalCategory,
        SignalDrain, SignalEvent, Volume,
    };

    fn vol(raw: u8) -> Volume {
        Volume::from_raw(raw)
    }

    fn volume_tick(from: u8, to: u8) -> GestureTick<VolumePayload> {
        GestureTick {
            context: context(),
            payload: VolumePayload {
                from: vol(from),
                to: vol(to),
                gesture: GestureTime::from_ms(0),
                input: InputSource::Mouse,
                section: Section::UNKNOWN,
                since_track_start: ElapsedDs::from_ds(100),
                since_section_start: ElapsedDs::from_ds(10),
            },
        }
    }

    fn system_tick(from: u8, to: u8) -> GestureTick<SystemVolumePayload> {
        GestureTick {
            context: context(),
            payload: SystemVolumePayload {
                from: vol(from),
                to: vol(to),
                gesture: GestureTime::from_ms(0),
                input: InputSource::System,
                section: Section::UNKNOWN,
                since_track_start: ElapsedDs::from_ds(0),
                since_section_start: ElapsedDs::from_ds(0),
                endpoint_class: OutputClass::Speakers,
            },
        }
    }

    fn rate_tick(from: u8, to: u8) -> GestureTick<RatePayload> {
        GestureTick {
            context: context(),
            payload: RatePayload {
                from: Rate::from_raw(from),
                to: Rate::from_raw(to),
                input: InputSource::Keyboard,
                section: Section::UNKNOWN,
            },
        }
    }

    fn seek_tick(from_s: u64, to_s: u64) -> GestureTick<SeekPayload> {
        seek_tick_ms(from_s * 1_000, to_s * 1_000)
    }

    fn seek_tick_ms(from_ms: u64, to_ms: u64) -> GestureTick<SeekPayload> {
        GestureTick {
            context: context(),
            payload: SeekPayload {
                from_ds: PositionDs::from_ms(from_ms),
                to_ds: PositionDs::from_ms(to_ms),
                section_from: Section::UNKNOWN,
                section_to: Section::UNKNOWN,
                input: InputSource::Mouse,
            },
        }
    }

    struct Rig {
        co: GestureCoalescer,
        drain: SignalDrain,
        sink: VecSink,
        clock: ManualClock,
    }

    impl Rig {
        fn new() -> Self {
            let (bus, drain, clock) = bus_with(|_| {});
            Self {
                co: GestureCoalescer::new(bus),
                drain,
                sink: VecSink::default(),
                clock,
            }
        }

        fn events(&mut self) -> Vec<SignalEvent> {
            self.sink.pump_all(&mut self.drain).to_vec()
        }
    }

    #[test]
    fn the_window_table_has_the_per_kind_values() {
        let windows: Vec<_> = SignalKind::ALL
            .iter()
            .filter_map(|&k| coalesce_window_ms(k).map(|w| (k, w)))
            .collect();
        assert_eq!(
            windows,
            [
                (SignalKind::VolumeChanged, 600),
                (SignalKind::SystemVolumeChanged, 400),
                (SignalKind::RateChanged, 600),
                (SignalKind::SeekForward, 2_000),
                (SignalKind::SeekBack, 2_000),
            ]
        );
        assert_eq!(VolumePayload::WINDOW_MS, 600);
        assert_eq!(RatePayload::WINDOW_MS, 600);
        assert_eq!(SystemVolumePayload::WINDOW_MS, 400);
        assert_eq!(SeekPayload::WINDOW_MS, 2_000);
        for (kind, payload_kind, window) in [
            (
                VolumePayload::TOGGLE_KIND,
                SignalKind::VolumeChanged,
                VOLUME_WINDOW_MS,
            ),
            (
                RatePayload::TOGGLE_KIND,
                SignalKind::RateChanged,
                RATE_WINDOW_MS,
            ),
            (
                SystemVolumePayload::TOGGLE_KIND,
                SignalKind::SystemVolumeChanged,
                SYSTEM_VOLUME_WINDOW_MS,
            ),
            (
                SeekPayload::TOGGLE_KIND,
                SignalKind::SeekBack,
                SEEK_WINDOW_MS,
            ),
        ] {
            assert_eq!(coalesce_window_ms(kind), Some(window));
            assert_eq!(
                (SignalCategory::of(kind), kind.tier()),
                (SignalCategory::of(payload_kind), payload_kind.tier()),
                "{kind} toggles like {payload_kind}"
            );
        }
        for kind in windows.iter().map(|&(k, _)| k) {
            assert!(kind.is_event(), "{kind}");
        }
    }

    #[test]
    fn forty_slider_ticks_in_500_ms_become_one_event_with_first_from_and_last_to() {
        let mut rig = Rig::new();
        for i in 0..40_u8 {
            let from = 100 + i;
            let outcome = rig.co.volume(|| volume_tick(from, from + 1));
            assert_eq!(outcome, TickOutcome::Buffered);
            if i < 39 {
                rig.clock.advance(12); // 39 * 12 = 468 ms
            }
        }
        assert_eq!(rig.co.flush_due(), 0, "still inside the window");
        assert!(rig.events().is_empty());
        rig.clock.advance(599);
        assert_eq!(rig.co.flush_due(), 0, "599 ms of silence is not enough");
        rig.clock.advance(1);
        assert_eq!(rig.co.flush_due(), 1);
        let events = rig.events();
        assert_eq!(events.len(), 1);
        let SignalPayload::VolumeChanged(p) = events[0].payload else {
            panic!("expected VolumeChanged, got {:?}", events[0].kind());
        };
        assert_eq!((p.from, p.to), (vol(100), vol(140)));
        assert_eq!(p.gesture, GestureTime::from_ms(468));
        assert_eq!(events[0].ts, UtcMillis::new(T0 + 468), "ts = last tick");
        assert!(rig.co.is_idle());
    }

    #[test]
    fn a_tick_after_the_window_starts_a_new_gesture() {
        let mut rig = Rig::new();
        let _ = rig.co.volume(|| volume_tick(10, 20));
        rig.clock.advance(600);
        let _ = rig.co.volume(|| volume_tick(20, 30));
        assert_eq!(
            rig.events().len(),
            1,
            "the first gesture was closed by the tick"
        );
        rig.clock.advance(600);
        assert_eq!(rig.co.flush_due(), 1);
        let events = rig.events();
        let froms: Vec<_> = events
            .iter()
            .map(|e| match e.payload {
                SignalPayload::VolumeChanged(p) => (p.from, p.to),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(froms, [(vol(10), vol(20)), (vol(20), vol(30))]);
    }

    #[test]
    fn each_tick_restarts_the_silence_window() {
        let mut rig = Rig::new();
        for _ in 0..5 {
            let _ = rig.co.volume(|| volume_tick(1, 2));
            rig.clock.advance(500);
            assert_eq!(rig.co.flush_due(), 0);
        }
        rig.clock.advance(100);
        assert_eq!(rig.co.flush_due(), 1);
        let events = rig.events();
        let SignalPayload::VolumeChanged(p) = events[0].payload else {
            panic!("kind");
        };
        assert_eq!(p.gesture.ms(), 2_000, "first to last tick");
    }

    #[test]
    fn a_single_press_to_release_tick_keeps_its_own_gesture_time() {
        let mut rig = Rig::new();
        let mut tick = volume_tick(50, 200);
        tick.payload.gesture = GestureTime::from_ms(3_000);
        let _ = rig.co.volume(|| tick);
        rig.clock.advance(600);
        assert_eq!(rig.co.flush_due(), 1);
        let SignalPayload::VolumeChanged(p) = rig.events()[0].payload else {
            panic!("kind");
        };
        assert_eq!(p.gesture.ms(), 3_000);
        assert_eq!((p.from, p.to), (vol(50), vol(200)));
    }

    #[test]
    fn the_last_tick_supplies_everything_but_from() {
        let mut rig = Rig::new();
        let _ = rig.co.volume(|| volume_tick(10, 20));
        rig.clock.advance(100);
        let mut last = volume_tick(20, 90);
        last.payload.input = InputSource::Keyboard;
        last.payload.since_track_start = ElapsedDs::from_ds(777);
        last.context.pos_ds = Some(4_321);
        let _ = rig.co.volume(|| last);
        rig.co.flush_all();
        let events = rig.events();
        let SignalPayload::VolumeChanged(p) = events[0].payload else {
            panic!("kind");
        };
        assert_eq!(p.from, vol(10));
        assert_eq!(p.to, vol(90));
        assert_eq!(p.input, InputSource::Keyboard);
        assert_eq!(p.since_track_start, ElapsedDs::from_ds(777));
        assert_eq!(events[0].pos_ds, Some(4_321));
    }

    #[test]
    fn a_gesture_that_returns_to_its_start_emits_nothing() {
        let mut rig = Rig::new();
        let _ = rig.co.volume(|| volume_tick(100, 150));
        rig.clock.advance(50);
        let _ = rig.co.volume(|| volume_tick(150, 100));
        rig.clock.advance(600);
        assert_eq!(rig.co.flush_due(), 0);
        assert!(rig.events().is_empty());
        assert_eq!(rig.co.suppressed(), 1);
    }

    #[test]
    fn system_volume_gestures_end_after_400_ms() {
        let mut rig = Rig::new();
        for i in 0..10_u8 {
            let _ = rig.co.system_volume(|| system_tick(i, i + 1));
            rig.clock.advance(20);
        }
        rig.clock.advance(379); // 399 ms since the last tick
        assert_eq!(rig.co.flush_due(), 0);
        rig.clock.advance(1);
        assert_eq!(rig.co.flush_due(), 1);
        let events = rig.events();
        let SignalPayload::SystemVolumeChanged(p) = events[0].payload else {
            panic!("kind");
        };
        assert_eq!((p.from, p.to), (vol(0), vol(10)));
        assert_eq!(p.endpoint_class, OutputClass::Speakers);
        assert_eq!(
            p.gesture.ms(),
            200,
            "first tick to last tick: 9 * 20 ms, quantized"
        );
    }

    #[test]
    fn rate_gestures_end_after_600_ms() {
        let mut rig = Rig::new();
        let _ = rig.co.rate(|| rate_tick(50, 55));
        rig.clock.advance(300);
        let _ = rig.co.rate(|| rate_tick(55, 60));
        rig.clock.advance(599);
        assert_eq!(rig.co.flush_due(), 0);
        rig.clock.advance(1);
        assert_eq!(rig.co.flush_due(), 1);
        let events = rig.events();
        let SignalPayload::RateChanged(p) = events[0].payload else {
            panic!("kind");
        };
        assert_eq!((p.from.raw(), p.to.raw()), (50, 60));
    }

    #[test]
    fn seeks_within_two_seconds_merge_into_one_with_first_from_and_last_to() {
        let mut rig = Rig::new();
        let _ = rig.co.seek(|| seek_tick(100, 110));
        rig.clock.advance(1_500);
        let _ = rig.co.seek(|| seek_tick(110, 125));
        rig.clock.advance(1_999);
        let _ = rig.co.seek(|| seek_tick(125, 140));
        rig.clock.advance(1_999);
        assert_eq!(rig.co.flush_due(), 0, "1 999 ms of silence");
        rig.clock.advance(1);
        assert_eq!(rig.co.flush_due(), 1);
        let events = rig.events();
        assert_eq!(events.len(), 1);
        let SignalPayload::SeekForward(p) = events[0].payload else {
            panic!("expected SeekForward, got {:?}", events[0].kind());
        };
        assert_eq!((p.from_ds.ms(), p.to_ds.ms()), (100_000, 140_000));
    }

    #[test]
    fn seeks_two_seconds_apart_stay_separate() {
        let mut rig = Rig::new();
        let _ = rig.co.seek(|| seek_tick(100, 120));
        rig.clock.advance(2_000);
        let _ = rig.co.seek(|| seek_tick(120, 90));
        rig.co.flush_all();
        let kinds: Vec<_> = rig.events().iter().map(SignalEvent::kind).collect();
        assert_eq!(kinds, [SignalKind::SeekForward, SignalKind::SeekBack]);
    }

    #[test]
    fn a_merged_seek_takes_its_kind_from_the_net_direction() {
        let mut rig = Rig::new();
        let _ = rig.co.seek(|| seek_tick(100, 130)); // forward 30 s
        rig.clock.advance(300);
        let _ = rig.co.seek(|| seek_tick(130, 60)); // then back 70 s: net back
        rig.co.flush_all();
        let events = rig.events();
        assert_eq!(events.len(), 1);
        let SignalPayload::SeekBack(p) = events[0].payload else {
            panic!("expected SeekBack");
        };
        assert_eq!((p.from_ds.ms(), p.to_ds.ms()), (100_000, 60_000));
    }

    #[test]
    fn a_net_seek_of_two_seconds_or_less_is_not_a_seek() {
        let mut rig = Rig::new();
        let _ = rig.co.seek(|| seek_tick_ms(100_000, 102_000));
        rig.co.flush_all();
        let _ = rig.co.seek(|| seek_tick_ms(100_000, 102_100));
        rig.co.flush_all();
        let _ = rig.co.seek(|| seek_tick(100, 140));
        rig.clock.advance(100);
        let _ = rig.co.seek(|| seek_tick(140, 101));
        rig.co.flush_all();
        let events = rig.events();
        assert_eq!(events.len(), 1, "only the 2.1 s seek counts");
        assert_eq!(events[0].kind(), SignalKind::SeekForward);
        assert_eq!(rig.co.suppressed(), 2);
    }

    #[test]
    fn a_disabled_category_never_builds_the_tick() {
        let mut rig = Rig::new();
        rig.co
            .bus
            .settings()
            .set_category(SignalCategory::Volume, false);
        let built = std::cell::Cell::new(0);
        for _ in 0..40 {
            let outcome = rig.co.volume(|| {
                built.set(built.get() + 1);
                volume_tick(1, 2)
            });
            assert_eq!(outcome, TickOutcome::Disabled);
        }
        rig.co.flush_all();
        assert_eq!(built.get(), 0);
        assert!(rig.events().is_empty());
        // Seeks use another category and are unaffected.
        let _ = rig.co.seek(|| seek_tick(0, 60));
        rig.co.flush_all();
        assert_eq!(rig.events().len(), 1);
    }

    #[test]
    fn a_category_turned_off_mid_gesture_drops_the_pending_event_unbuilt() {
        let mut rig = Rig::new();
        let _ = rig.co.volume(|| volume_tick(1, 50));
        rig.co
            .bus
            .settings()
            .set_category(SignalCategory::Volume, false);
        assert_eq!(rig.co.flush_all(), 0);
        assert!(rig.events().is_empty());
    }

    #[test]
    fn a_tick_for_another_recording_ends_the_gesture_first() {
        let mut rig = Rig::new();
        let _ = rig.co.volume(|| volume_tick(10, 20));
        rig.clock.advance(10);
        let mut other = volume_tick(20, 30);
        other.context.recording = Some(recording(9));
        let _ = rig.co.volume(|| other);
        rig.co.flush_all();
        let events = rig.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].recording, Some(recording(3)));
        assert_eq!(events[1].recording, Some(recording(9)));
    }

    #[test]
    fn kinds_coalesce_independently() {
        let mut rig = Rig::new();
        let _ = rig.co.volume(|| volume_tick(1, 2));
        let _ = rig.co.rate(|| rate_tick(50, 60));
        rig.clock.advance(400);
        let _ = rig.co.system_volume(|| system_tick(5, 6));
        rig.clock.advance(200); // volume/rate at 600 ms, system at 200 ms
        assert_eq!(rig.co.flush_due(), 2);
        assert!(!rig.co.is_idle());
        assert_eq!(rig.co.next_deadline(), Some(UtcMillis::new(T0 + 400 + 400)));
    }

    #[test]
    fn next_deadline_is_the_earliest_silence_end() {
        let mut rig = Rig::new();
        assert_eq!(rig.co.next_deadline(), None);
        let _ = rig.co.seek(|| seek_tick(0, 60));
        assert_eq!(rig.co.next_deadline(), Some(UtcMillis::new(T0 + 2_000)));
        let _ = rig.co.volume(|| volume_tick(1, 2));
        assert_eq!(rig.co.next_deadline(), Some(UtcMillis::new(T0 + 600)));
    }

    #[test]
    fn a_clock_that_goes_backwards_ends_the_gesture_instead_of_hanging_it() {
        let mut rig = Rig::new();
        let _ = rig.co.volume(|| volume_tick(1, 2));
        rig.clock.advance(-3_600_000);
        assert_eq!(rig.co.flush_due(), 1);
    }

    #[test]
    fn flush_all_and_drop_emit_what_is_pending() {
        let (bus, mut drain, _clock) = bus_with(|_| {});
        let mut co = GestureCoalescer::new(bus);
        let _ = co.volume(|| volume_tick(1, 9));
        let _ = co.seek(|| seek_tick(0, 60));
        drop(co);
        let mut sink = VecSink::default();
        let kinds: Vec<_> = sink
            .pump_all(&mut drain)
            .iter()
            .map(SignalEvent::kind)
            .collect();
        assert_eq!(kinds, [SignalKind::VolumeChanged, SignalKind::SeekForward]);
    }

    #[test]
    fn coalesced_events_get_consecutive_seqs_only_when_emitted() {
        let (bus, mut drain, clock) = bus_with(|c| c.first_seq = 10);
        let mut co = GestureCoalescer::new(bus);
        for i in 0..40_u8 {
            let _ = co.volume(|| volume_tick(i, i + 1));
            clock.advance(10);
        }
        co.flush_all();
        let mut sink = VecSink::default();
        let seqs: Vec<_> = sink.pump_all(&mut drain).iter().map(|e| e.id.seq).collect();
        assert_eq!(seqs, [10], "40 ticks consumed one seq");
    }
}
