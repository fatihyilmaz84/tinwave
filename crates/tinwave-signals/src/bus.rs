//! The signal bus: a bounded, non-blocking path from producers to the signals.db writer.
//!
//! # Shape
//!
//! [`SignalBus::channel`] returns two ends. [`SignalBus`] is the producer handle: cheap to
//! clone, `Send + Sync`, held by the playback engine, the view models and the context
//! providers. [`SignalDrain`] is the single consumer, owned by the writer thread, which calls
//! [`SignalDrain::pump`] with a [`SignalSink`].
//!
//! No async runtime is used: the producer side is plain synchronous code (the audio and UI
//! threads must not depend on an executor) and the consumer is one dedicated thread that blocks
//! in [`SignalDrain::wait`]. `docs/rules/rust.md` asks for bounded `crossbeam_channel`s; this
//! module uses two.
//!
//! # Policy
//!
//! - **Producers never block.** `emit_with` does an atomic check, a clock read, one `try_send`
//!   into a pre-allocated array and, when full, an eviction. Nothing allocates.
//! - **Toggles at the source.** `emit_with` takes a constructor closure and only calls it when
//!   [`SignalSettings::enabled`] says the kind is collected.
//! - **Two queues** ([`Priority`]). [`Priority::Reserved`] events have their own queue. When the
//!   low queue is full the producer evicts the *oldest* low event (any producer can: the
//!   channel is multi-consumer), counts it in the per-kind drop table and retries.
//! - **Reserved overflow is a bug, not a policy.** Reserved events are never evicted. If the
//!   reserved queue is full (the writer is stalled for longer than the queue can absorb) the new
//!   event is rejected, [`EmitOutcome::Dropped`] is returned and
//!   [`SignalBus::reserved_overflows`] goes up; the Diagnostics page must show a non-zero value
//!   as an error. The default capacity is two orders of magnitude above the reference user's
//!   rate (about 400 reserved events a day).
//!
//! # Ordering
//!
//! Each emitted event gets the next `seq` of the bus's device (`SignalEventId`). `seq` is unique
//! and increases in allocation order. [`SignalDrain::pump`] merges the two queues in `seq`
//! order, so events leave in the order they were emitted. The one exception is two producer
//! threads that allocate `seq`s within microseconds of each other and enqueue in the opposite
//! order, which a batch boundary can split. Nothing relies on queue order: rowid order is only
//! "approximately time order" (`docs/rules/storage-budget.md` section 3).
//!
//! # `seq` has gaps and commits slightly out of order
//!
//! `seq` is **not** a gap-free counter. A `seq` is spent whenever an event is built, and some
//! of those events never reach the sink:
//!
//! - a low-priority event evicted by a newer one ([`SignalBus::dropped`]);
//! - a low-priority event lost under contention after a few eviction attempts
//!   ([`SignalBus::dropped`]);
//! - a reserved event rejected because the reserved queue was full
//!   ([`SignalBus::reserved_overflows`]);
//! - an event whose constructor returned a payload of another kind
//!   ([`SignalsError::KindMismatch`]);
//! - events still in the queues when the process dies.
//!
//! And because two producer threads can enqueue in the opposite order to their `seq`
//! allocation, a batch can commit `seq` 11 before a later batch commits `seq` 10. Therefore:
//!
//! - **A gap in the stored `seq`s is not a loss.** The writer, the diagnostics and any
//!   integrity check must not report one, and must not try to fill it.
//! - **"Highest `seq` seen" is not a cursor.** A sync cursor or a "continue from here" marker
//!   built on the maximum committed `seq` skips an event that commits after it. Dedupe by the
//!   exact `(device, seq)` key (that is what the key is for); a sync protocol needs its own
//!   watermark that tolerates late arrivals.
//! - The one legitimate use of the highest stored `seq` is [`BusConfig::first_seq`] at
//!   start-up: one more than the maximum, so a new run cannot reuse a `seq`.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Select, Sender, TryRecvError, TrySendError};
use tinwave_core::{Clock, DeviceId, SignalEventId, TzOffsetMin, UtcMillis};

use crate::coalesce::EventContext;
use crate::counters::{CounterRow, UiCounters};
use crate::derived::DerivedValue;
use crate::priority::Priority;
use crate::{SignalEvent, SignalKind, SignalPayload, SignalSettings, SignalsError, StorageClass};

/// Where drained signals go. Implemented by the signals.db writer (T-0006).
///
/// The bus calls it from one thread, with batches.
///
/// # Contract: all or nothing
///
/// One call to [`write`](Self::write) **MUST** be exactly one transaction covering the events,
/// the counter rows and everything the sink derives from them (the dictionary entries it creates
/// to resolve counter entities, for example). It returns `Ok` only after that transaction
/// committed, and returns `Err` only if **nothing** was committed. There is no third outcome.
///
/// The bus relies on it: after an `Err` it re-offers the same events and the same counter rows.
/// Events are safe to re-offer because `(device, seq)` is a dedupe key. **Counter rows are not
/// idempotent**: they are deltas that get added, so a sink that committed the counters (or some
/// of the rows) and then returned `Err` would have them added a second time by the retry, and
/// the day's counts would be too high for good.
pub trait SignalSink {
    /// Persists a batch.
    ///
    /// - `events` are raw events in ascending `seq` order. Insert them with `INSERT OR IGNORE`
    ///   on `(device, seq)`. Their `seq`s have gaps and need not be above everything stored
    ///   already (see the module docs); neither is an error.
    /// - `counters` are deltas since the previous successful call. Resolve each row's UUID to
    ///   its dictionary id (`dict_recording` for [`EntityKind::Track`](crate::EntityKind::Track), `dict_entity`
    ///   otherwise), then **add** `count` and `total_ms` to the existing
    ///   `(day, kind, entity_kind, entity)` row (an `INSERT .. ON CONFLICT DO UPDATE`) and keep
    ///   the larger `max_scrolled`.
    ///
    /// # Errors
    /// On `Err` nothing has been committed (see the contract above). The bus keeps the batch
    /// and offers the same events and counter rows again on the next [`SignalDrain::pump`].
    /// Callers back off between attempts: [`SignalDrain::wait`] does so while a batch is
    /// retained.
    fn write(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
    ) -> Result<(), SignalsError>;

    /// Persists a batch that may also hold derived values. This is the method the drain calls;
    /// everything said about [`write`](Self::write) holds for it, with `derived` part of the one
    /// transaction.
    ///
    /// - `derived` are in ascending `seq` order and, merged with `events` by `seq`, never put a
    ///   value before an event that was emitted before it, or after one emitted after it: a batch
    ///   is a prefix of the merged order. A value emitted just before the event that ends its play
    ///   arrives in the same batch as that event or in the one before. Store the events first and
    ///   the values after them, so a value finds the record its batch's outcome makes.
    /// - A value that finds nothing to attach to (its play has no record) is **not** an error: the
    ///   play may be one the listener did not allow to be recorded. A sink counts it.
    ///
    /// The default forwards to [`write`](Self::write) and discards `derived`, for sinks that have
    /// nowhere to keep a derived value (a test collector, a probe). A sink that stores plays must
    /// override it: the writer's store does.
    ///
    /// # Errors
    /// As [`write`](Self::write): on `Err` nothing has been committed, and the bus offers the same
    /// events, counter rows and values again.
    fn write_with_derived(
        &mut self,
        events: &[SignalEvent],
        counters: &[CounterRow],
        derived: &[DerivedValue],
    ) -> Result<(), SignalsError> {
        let _ = derived;
        self.write(events, counters)
    }
}

/// Sizes and identity of a bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusConfig {
    /// This installation; every event id carries it.
    pub device: DeviceId,
    /// First `seq` to hand out: one more than the highest stored `seq` of this device.
    pub first_seq: u64,
    /// Timezone offset used for counter days until a session sets another.
    pub tz_offset: TzOffsetMin,
    /// Slots in the reserved queue. Memory: this times `size_of::<SignalEvent>()`.
    pub reserved_capacity: usize,
    /// Slots in the low-priority queue.
    pub low_capacity: usize,
    /// Distinct `(day, kind, entity_kind, entity)` rows the counters hold between drains.
    pub max_counter_keys: usize,
    /// Most events plus derived values handed to the sink in one call.
    pub max_batch: usize,
    /// Slots in the derived-value queue. One value per heard play, so this is two orders of
    /// magnitude above the reference user's rate (about 120 a day).
    pub derived_capacity: usize,
}

impl BusConfig {
    /// Defaults for `device`: 2 048 reserved and 512 low slots (about 0.35 MB), 16 384 counter
    /// keys, batches of 256 events (the batch size the write budget in `testing.md` uses).
    #[must_use]
    pub const fn new(device: DeviceId) -> Self {
        Self {
            device,
            first_seq: 0,
            tz_offset: TzOffsetMin::UTC,
            reserved_capacity: 2_048,
            low_capacity: 512,
            max_counter_keys: 16_384,
            max_batch: 256,
            derived_capacity: 256,
        }
    }

    fn validate(&self) -> Result<(), SignalsError> {
        let positive = [
            (
                self.reserved_capacity,
                "reserved_capacity must be at least 1",
            ),
            (self.low_capacity, "low_capacity must be at least 1"),
            (self.max_counter_keys, "max_counter_keys must be at least 1"),
            (self.max_batch, "max_batch must be at least 1"),
            (self.derived_capacity, "derived_capacity must be at least 1"),
        ];
        for (value, what) in positive {
            if value == 0 {
                return Err(SignalsError::InvalidConfig { what });
            }
        }
        Ok(())
    }
}

/// The identity and time the bus assigned to an event about to be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventStamp {
    /// `(device, seq)` of the event. Put it in `SignalEvent::id` unchanged.
    pub id: SignalEventId,
    /// The injected clock's time now. Normally `SignalEvent::ts`.
    pub ts: UtcMillis,
}

/// What happened to an event passed to [`SignalBus::emit_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum EmitOutcome {
    /// The event is queued. (An older low-priority event may have been evicted to make room;
    /// that is counted in [`SignalBus::dropped`].)
    Queued,
    /// The kind's category or tier is off: the constructor did not run.
    Disabled,
    /// The event was built but could not be queued and was lost (reserved queue full, or the
    /// low queue stayed full under contention). Counted in [`SignalBus::dropped`]; reserved
    /// losses also in [`SignalBus::reserved_overflows`].
    Dropped,
}

/// Attempts to make room in the low queue before giving up on the new event.
const MAX_EVICTIONS_PER_EMIT: usize = 4;

/// State shared by producers and the drain.
struct Shared {
    clock: Arc<dyn Clock>,
    settings: Arc<SignalSettings>,
    counters: UiCounters,
    /// Dropped events per kind discriminant.
    drops: [AtomicU64; 256],
    reserved_overflows: AtomicU64,
    queued: AtomicU64,
    /// Derived values accepted into their queue. Not part of `queued`: a consumer that waits for
    /// `queued` events to be stored would wait for values that are not events.
    derived_queued: AtomicU64,
    /// Derived values that were queued while their kind was collected and dropped when it left
    /// the queue because it no longer was.
    derived_withdrawn: AtomicU64,
}

struct Inner {
    shared: Arc<Shared>,
    device: DeviceId,
    next_seq: AtomicU64,
    reserved_tx: Sender<SignalEvent>,
    low_tx: Sender<SignalEvent>,
    derived_tx: Sender<DerivedValue>,
    /// A consumer handle on the low queue, used only to evict its oldest event.
    low_evict: Receiver<SignalEvent>,
}

/// The producer handle of the bus. Clone it freely.
#[derive(Clone)]
pub struct SignalBus {
    inner: Arc<Inner>,
}

impl fmt::Debug for SignalBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignalBus")
            .field("device", &self.inner.device)
            .field("queued", &self.queued_total())
            .field("dropped", &self.dropped_total())
            .field("reserved_overflows", &self.reserved_overflows())
            .finish_non_exhaustive()
    }
}

impl SignalBus {
    /// Creates a bus and its drain.
    ///
    /// `clock` is the injected time source (tests pass a `ManualClock`); `settings` are the
    /// shared Signal Center toggles.
    ///
    /// # Errors
    /// [`SignalsError::InvalidConfig`] if a capacity is zero.
    pub fn channel(
        config: BusConfig,
        clock: impl Clock + 'static,
        settings: Arc<SignalSettings>,
    ) -> Result<(Self, SignalDrain), SignalsError> {
        config.validate()?;
        let clock: Arc<dyn Clock> = Arc::new(clock);
        let counters = UiCounters::new(
            clock.clone(),
            settings.clone(),
            config.tz_offset,
            config.max_counter_keys,
        );
        let shared = Arc::new(Shared {
            clock,
            settings,
            counters,
            drops: std::array::from_fn(|_| AtomicU64::new(0)),
            reserved_overflows: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            derived_queued: AtomicU64::new(0),
            derived_withdrawn: AtomicU64::new(0),
        });
        let (reserved_tx, reserved_rx) = crossbeam_channel::bounded(config.reserved_capacity);
        let (low_tx, low_rx) = crossbeam_channel::bounded(config.low_capacity);
        let (derived_tx, derived_rx) = crossbeam_channel::bounded(config.derived_capacity);
        let bus = Self {
            inner: Arc::new(Inner {
                shared: shared.clone(),
                device: config.device,
                next_seq: AtomicU64::new(config.first_seq),
                reserved_tx,
                low_tx,
                derived_tx,
                low_evict: low_rx.clone(),
            }),
        };
        let drain = SignalDrain {
            shared,
            reserved: Lane::new(reserved_rx),
            low: Lane::new(low_rx),
            derived_lane: Lane::new(derived_rx),
            max_batch: config.max_batch,
            events: Vec::with_capacity(config.max_batch),
            derived: Vec::new(),
            counter_rows: Vec::new(),
        };
        Ok((bus, drain))
    }

    /// Whether events of `kind` are collected. Check this *before* doing any work to build an
    /// event; [`emit_with`](Self::emit_with) does it for you.
    #[must_use]
    pub fn enabled(&self, kind: SignalKind) -> bool {
        self.inner.shared.settings.enabled(kind)
    }

    /// The shared toggles.
    #[must_use]
    pub fn settings(&self) -> &Arc<SignalSettings> {
        &self.inner.shared.settings
    }

    /// The UI counters (`Impression`, `ArtworkHover`, `PageDwell`).
    #[must_use]
    pub fn counters(&self) -> &UiCounters {
        &self.inner.shared.counters
    }

    /// The bus's clock reading, UTC.
    #[must_use]
    pub fn now(&self) -> UtcMillis {
        self.inner.shared.clock.now()
    }

    /// The device every event id carries.
    #[must_use]
    pub fn device(&self) -> DeviceId {
        self.inner.device
    }

    /// Sets the timezone offset used for counter days (call when a session starts).
    pub fn set_tz_offset(&self, tz: TzOffsetMin) {
        self.inner.shared.counters.set_tz_offset(tz);
    }

    /// Emits an event of `kind`, building it only if the kind is collected.
    ///
    /// `build` receives the stamp (id and time) and must return an event whose payload is of
    /// `kind`. It is **not called** when the category or tier is off, so disabled signals cost
    /// no construction at all. Never blocks.
    ///
    /// # Errors
    /// - [`SignalsError::NotAnEvent`] if `kind` is a counter or derived kind (use
    ///   [`counters`](Self::counters) for counters).
    /// - [`SignalsError::KindMismatch`] if `build` returned a payload of another kind. The
    ///   event is discarded: it was checked against the wrong toggle.
    pub fn emit_with(
        &self,
        kind: SignalKind,
        build: impl FnOnce(EventStamp) -> SignalEvent,
    ) -> Result<EmitOutcome, SignalsError> {
        if !kind.is_event() {
            return Err(SignalsError::NotAnEvent(kind));
        }
        let shared = &self.inner.shared;
        if !shared.settings.enabled(kind) {
            return Ok(EmitOutcome::Disabled);
        }
        let stamp = EventStamp {
            id: SignalEventId::new(
                self.inner.device,
                self.inner.next_seq.fetch_add(1, Ordering::Relaxed),
            ),
            ts: shared.clock.now(),
        };
        let event = build(stamp);
        if event.kind() != kind {
            return Err(SignalsError::KindMismatch {
                expected: kind,
                actual: event.kind(),
            });
        }
        Ok(self.enqueue(event))
    }

    /// Emits a derived value of `kind`, building it only if the kind is collected.
    ///
    /// The way a derived signal (storage class `D`) reaches the writer: it is not a
    /// `signal_event`, so [`emit_with`](Self::emit_with) refuses it. `build` receives the stamp
    /// (the `seq` that orders the value among the events, and the time) and is **not called** when
    /// the kind is off. A derived kind is enabled only while its own category and tier **and the
    /// category and tier of every input** are on ([`SignalSettings::enabled`]). Never blocks.
    ///
    /// **Emit the value just before the event that ends the play it describes**: the value takes
    /// the next `seq`, and the drain hands values and events over in `seq` order, so the two are
    /// in one batch (see the module docs of `derived`) and the writer stores both in the
    /// transaction of the play-end event.
    ///
    /// A full queue (the writer has stalled for a very long time) loses the new value, which is
    /// counted in [`dropped`](Self::dropped) under its kind and returns [`EmitOutcome::Dropped`].
    ///
    /// # Errors
    /// - [`SignalsError::NotDerived`] if `kind` is a raw event or a counter.
    /// - [`SignalsError::KindMismatch`] if `build` returned a value of another kind. The value is
    ///   discarded: it was checked against the wrong toggle.
    pub fn emit_derived(
        &self,
        kind: SignalKind,
        build: impl FnOnce(EventStamp) -> DerivedValue,
    ) -> Result<EmitOutcome, SignalsError> {
        if kind.storage_class() != StorageClass::Derived {
            return Err(SignalsError::NotDerived(kind));
        }
        let shared = &self.inner.shared;
        if !shared.settings.enabled(kind) {
            return Ok(EmitOutcome::Disabled);
        }
        let stamp = EventStamp {
            id: SignalEventId::new(
                self.inner.device,
                self.inner.next_seq.fetch_add(1, Ordering::Relaxed),
            ),
            ts: shared.clock.now(),
        };
        let value = build(stamp);
        if value.kind() != kind {
            return Err(SignalsError::KindMismatch {
                expected: kind,
                actual: value.kind(),
            });
        }
        match self.inner.derived_tx.try_send(value) {
            Ok(()) => {
                shared.derived_queued.fetch_add(1, Ordering::Relaxed);
                Ok(EmitOutcome::Queued)
            }
            Err(TrySendError::Full(lost) | TrySendError::Disconnected(lost)) => {
                self.count_drop(lost.kind());
                Ok(EmitOutcome::Dropped)
            }
        }
    }

    /// Emits a finished payload (used by the coalescer): infallible because the kind is the
    /// payload's own, so it is always an event kind and always matches.
    pub(crate) fn emit_payload(
        &self,
        payload: SignalPayload,
        ts: UtcMillis,
        context: EventContext,
    ) -> EmitOutcome {
        let kind = payload.kind();
        let shared = &self.inner.shared;
        if !shared.settings.enabled(kind) {
            return EmitOutcome::Disabled;
        }
        let id = SignalEventId::new(
            self.inner.device,
            self.inner.next_seq.fetch_add(1, Ordering::Relaxed),
        );
        self.enqueue(SignalEvent {
            id,
            ts,
            session: context.session,
            recording: context.recording,
            source: context.source,
            pos_ds: context.pos_ds,
            payload,
            ctx: context.ctx,
        })
    }

    fn enqueue(&self, event: SignalEvent) -> EmitOutcome {
        let outcome = match Priority::of(event.kind()) {
            Priority::Reserved => self.enqueue_reserved(event),
            Priority::Low => self.enqueue_low(event),
        };
        if outcome == EmitOutcome::Queued {
            self.inner.shared.queued.fetch_add(1, Ordering::Relaxed);
        }
        outcome
    }

    fn enqueue_reserved(&self, event: SignalEvent) -> EmitOutcome {
        match self.inner.reserved_tx.try_send(event) {
            Ok(()) => EmitOutcome::Queued,
            Err(TrySendError::Full(lost) | TrySendError::Disconnected(lost)) => {
                self.inner
                    .shared
                    .reserved_overflows
                    .fetch_add(1, Ordering::Relaxed);
                self.count_drop(lost.kind());
                EmitOutcome::Dropped
            }
        }
    }

    fn enqueue_low(&self, event: SignalEvent) -> EmitOutcome {
        let mut pending = event;
        for _ in 0..MAX_EVICTIONS_PER_EMIT {
            match self.inner.low_tx.try_send(pending) {
                Ok(()) => return EmitOutcome::Queued,
                Err(TrySendError::Full(back)) => {
                    pending = back;
                    if let Ok(oldest) = self.inner.low_evict.try_recv() {
                        self.count_drop(oldest.kind());
                    }
                }
                Err(TrySendError::Disconnected(lost)) => {
                    self.count_drop(lost.kind());
                    return EmitOutcome::Dropped;
                }
            }
        }
        // Other producers refilled the queue every time we made room: lose the newest.
        self.count_drop(pending.kind());
        EmitOutcome::Dropped
    }

    fn count_drop(&self, kind: SignalKind) {
        let slot = &self.inner.shared.drops[usize::from(kind.discriminant())];
        slot.fetch_add(1, Ordering::Relaxed);
    }

    /// Events of `kind` lost to overflow since the bus was created (the in-memory
    /// `SignalDropped` diagnostic of the catalog).
    #[must_use]
    pub fn dropped(&self, kind: SignalKind) -> u64 {
        self.inner.shared.drops[usize::from(kind.discriminant())].load(Ordering::Relaxed)
    }

    /// All events lost to overflow.
    #[must_use]
    pub fn dropped_total(&self) -> u64 {
        self.inner
            .shared
            .drops
            .iter()
            .map(|d| d.load(Ordering::Relaxed))
            .sum()
    }

    /// Every kind with a non-zero drop count, in discriminant order.
    #[must_use]
    pub fn dropped_by_kind(&self) -> Vec<(SignalKind, u64)> {
        SignalKind::ALL
            .iter()
            .map(|&kind| (kind, self.dropped(kind)))
            .filter(|&(_, count)| count > 0)
            .collect()
    }

    /// Reserved (playback) events lost because the reserved queue was full. Must stay 0.
    #[must_use]
    pub fn reserved_overflows(&self) -> u64 {
        self.inner.shared.reserved_overflows.load(Ordering::Relaxed)
    }

    /// Events accepted into a queue since the bus was created. Derived values are not events and
    /// are counted by [`derived_queued_total`](Self::derived_queued_total).
    #[must_use]
    pub fn queued_total(&self) -> u64 {
        self.inner.shared.queued.load(Ordering::Relaxed)
    }

    /// Derived values accepted into their queue since the bus was created.
    #[must_use]
    pub fn derived_queued_total(&self) -> u64 {
        self.inner.shared.derived_queued.load(Ordering::Relaxed)
    }

    /// Derived values dropped on leaving the queue because their kind was no longer collected
    /// (the listener switched the category or tier off after they were queued).
    #[must_use]
    pub fn derived_withdrawn(&self) -> u64 {
        self.inner.shared.derived_withdrawn.load(Ordering::Relaxed)
    }
}

/// What one [`SignalDrain::pump`] call moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PumpReport {
    /// Events written to the sink.
    pub events: usize,
    /// Counter rows written to the sink.
    pub counter_rows: usize,
    /// Derived values written to the sink.
    pub derived: usize,
    /// More events were already waiting: call `pump` again without waiting.
    pub more: bool,
    /// Every producer handle is gone and all queues are empty: the writer can stop.
    pub closed: bool,
}

/// What a lane holds: something with a place in the `seq` order of the bus.
trait Sequenced {
    fn seq(&self) -> u64;
}

impl Sequenced for SignalEvent {
    fn seq(&self) -> u64 {
        self.id.seq
    }
}

impl Sequenced for DerivedValue {
    fn seq(&self) -> u64 {
        self.id.seq
    }
}

/// One of the queues, with the item pulled from it but not yet handed out.
///
/// A channel cannot be peeked, so merging the queues in `seq` order needs a one-item look-ahead
/// per queue.
struct Lane<T> {
    rx: Receiver<T>,
    head: Option<T>,
    /// `false` once the channel reported `Disconnected` (all producers gone, queue empty).
    open: bool,
}

impl<T: Sequenced> Lane<T> {
    fn new(rx: Receiver<T>) -> Self {
        Self {
            rx,
            head: None,
            open: true,
        }
    }

    /// The `seq` of the next item, pulling it from the channel if needed.
    fn peek_seq(&mut self) -> Option<u64> {
        if self.head.is_none() && self.open {
            match self.rx.try_recv() {
                Ok(item) => self.head = Some(item),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.open = false,
            }
        }
        self.head.as_ref().map(Sequenced::seq)
    }

    fn has_work(&self) -> bool {
        self.head.is_some() || !self.rx.is_empty()
    }

    fn queued(&self) -> usize {
        usize::from(self.head.is_some()) + self.rx.len()
    }
}

/// The consumer end of the bus, owned by the writer thread.
pub struct SignalDrain {
    shared: Arc<Shared>,
    reserved: Lane<SignalEvent>,
    low: Lane<SignalEvent>,
    derived_lane: Lane<DerivedValue>,
    max_batch: usize,
    /// The batch taken from the queues and not yet written (kept across a failed write).
    events: Vec<SignalEvent>,
    derived: Vec<DerivedValue>,
    counter_rows: Vec<CounterRow>,
}

impl fmt::Debug for SignalDrain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignalDrain")
            .field("max_batch", &self.max_batch)
            .field("retained_events", &self.events.len())
            .field("retained_derived", &self.derived.len())
            .field("retained_counter_rows", &self.counter_rows.len())
            .finish_non_exhaustive()
    }
}

impl SignalDrain {
    /// Blocks until an event is queued or `timeout` passes. Returns `true` if there is
    /// something to pump (queued events, or a batch retained after a failed write).
    ///
    /// **Retry backoff.** While a batch is retained because the sink failed, this always blocks
    /// for the full `timeout` and then returns `true`, whatever is queued: the timeout is the
    /// retry interval. Returning at once would make a writer loop (`wait`, `pump`, repeat) spin
    /// at full speed for as long as the database keeps failing. Pass the writer's flush interval
    /// (seconds), not a few milliseconds.
    ///
    /// **Shutdown.** That wait sleeps through producers disconnecting: while a failed batch is
    /// retained, quitting the app can be delayed by up to one flush interval. This only happens
    /// while the database keeps failing; with a healthy sink the wait returns as soon as the
    /// bus has work.
    ///
    /// Counter rows do not wake the drain: they are collected on every pump, so call `pump` at
    /// least every few seconds (the writer's flush interval) even when this returns `false`.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> bool {
        if self.has_retained_batch() {
            // `pump` clears the batch on success, so a batch that is still here failed to write.
            std::thread::sleep(timeout);
            return true;
        }
        if self.has_queued_work() {
            return true;
        }
        let mut select = Select::new();
        select.recv(&self.reserved.rx);
        select.recv(&self.low.rx);
        select.recv(&self.derived_lane.rx);
        select.ready_timeout(timeout).is_ok()
    }

    /// Whether a batch (events, derived values and/or counter rows) is waiting for a retry after
    /// a failed write.
    fn has_retained_batch(&self) -> bool {
        !self.events.is_empty() || !self.derived.is_empty() || !self.counter_rows.is_empty()
    }

    fn has_queued_work(&self) -> bool {
        self.reserved.has_work() || self.low.has_work() || self.derived_lane.has_work()
    }

    /// Events and derived values waiting in the queues (not counting a retained batch).
    #[must_use]
    pub fn queued(&self) -> usize {
        self.reserved.queued() + self.low.queued() + self.derived_lane.queued()
    }

    /// Whether `kind` is collected right now: the check the drain makes on a derived value as it
    /// leaves the queue, for a consumer that holds values longer (the writer's stage) and has to
    /// make it again before it stores them.
    #[must_use]
    pub fn enabled(&self, kind: SignalKind) -> bool {
        self.shared.settings.enabled(kind)
    }

    /// Moves up to `max_batch` events and derived values plus all aggregated counter rows to
    /// `sink`.
    ///
    /// Events and derived values leave in `seq` order across all queues. A derived value whose
    /// kind is no longer collected is dropped as it leaves the queue
    /// ([`SignalBus::derived_withdrawn`]). Does nothing (and does not call the
    /// sink) when there is nothing to write. If the sink fails the batch is retained and
    /// offered again by the next call, so a transient database error loses nothing; meanwhile
    /// the queues keep applying their drop policy, so memory stays bounded.
    ///
    /// # Errors
    /// Whatever the sink returned.
    pub fn pump<S: SignalSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<PumpReport, SignalsError> {
        if self.events.is_empty() && self.derived.is_empty() {
            self.fill();
        }
        if self.counter_rows.is_empty() {
            self.shared.counters.drain_into(&mut self.counter_rows);
        }
        let more = self.has_queued_work();
        let closed = !more && !self.reserved.open && !self.low.open && !self.derived_lane.open;
        if !self.has_retained_batch() {
            return Ok(PumpReport {
                events: 0,
                counter_rows: 0,
                derived: 0,
                more,
                closed,
            });
        }
        sink.write_with_derived(&self.events, &self.counter_rows, &self.derived)?;
        let report = PumpReport {
            events: self.events.len(),
            counter_rows: self.counter_rows.len(),
            derived: self.derived.len(),
            more,
            closed,
        };
        self.events.clear();
        self.derived.clear();
        self.counter_rows.clear();
        Ok(report)
    }

    /// Merges the queues into `events` and `derived` by `seq`, up to `max_batch` items in all.
    ///
    /// The derived lane is looked at **before** the event lanes in every round. A producer thread
    /// sends in `seq` order, so once a value has been received every event that producer sent
    /// before it (lower `seq`) is visible to the peeks that follow: a value is never taken while
    /// an event with a lower `seq` is still on its way, and the batch is a prefix of the merged
    /// order. (The controller sends a play's value just before its outcome event.) A value taken is dropped, and counted, if its kind is no longer collected.
    fn fill(&mut self) {
        while self.events.len() + self.derived.len() < self.max_batch {
            let next_derived = self.derived_lane.peek_seq();
            let next_event = match (self.reserved.peek_seq(), self.low.peek_seq()) {
                (Some(r), Some(l)) => Some(r.min(l)),
                (r, l) => r.or(l),
            };
            match (next_derived, next_event) {
                (Some(d), Some(e)) if d < e => self.take_derived(),
                (Some(_), None) => self.take_derived(),
                (_, Some(_)) => self.take_event(),
                (None, None) => break,
            }
        }
        // Producers on different threads can enqueue slightly out of `seq` order; this keeps a
        // batch ordered whatever the interleaving.
        self.events.sort_unstable_by_key(|e| e.id.seq);
        self.derived.sort_unstable_by_key(|d| d.id.seq);
    }

    /// Moves the head of the lower-`seq` event lane into the batch.
    fn take_event(&mut self) {
        let lane = match (self.reserved.peek_seq(), self.low.peek_seq()) {
            (Some(r), Some(l)) if r <= l => &mut self.reserved,
            (Some(_), None) => &mut self.reserved,
            (_, Some(_)) => &mut self.low,
            (None, None) => return,
        };
        self.events.extend(lane.head.take());
    }

    /// Moves the head of the derived lane into the batch, or drops it when it is no longer
    /// collected.
    fn take_derived(&mut self) {
        let Some(value) = self.derived_lane.head.take() else {
            return;
        };
        if self.shared.settings.enabled(value.kind()) {
            self.derived.push(value);
        } else {
            self.shared
                .derived_withdrawn
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use tinwave_core::ManualClock;

    use super::*;
    use crate::test_support::{bus_with as bus, device, event, recording, session, VecSink, T0};
    use crate::{
        CounterEntity, DerivedPayload, EntityKind, InputSource, PausedPayload, PlayRef, Rate,
        RateSessionPayload, SignalCategory, SignalPayload, View, ViewOpenedPayload,
    };
    use tinwave_core::Tier;

    fn track(n: u8) -> CounterEntity {
        CounterEntity::new(EntityKind::Track, [n; 16])
    }

    /// A database that is down: every write fails and counts the attempt.
    #[derive(Default)]
    struct DeadSink {
        attempts: usize,
    }

    impl SignalSink for DeadSink {
        fn write(&mut self, _: &[SignalEvent], _: &[CounterRow]) -> Result<(), SignalsError> {
            self.attempts += 1;
            Err(SignalsError::Sink {
                reason: "database is down".into(),
            })
        }
    }

    fn paused() -> SignalPayload {
        SignalPayload::Paused(PausedPayload {
            input: InputSource::Mouse,
        })
    }

    fn view_opened() -> SignalPayload {
        SignalPayload::ViewOpened(ViewOpenedPayload { view: View::Lyrics })
    }

    #[test]
    fn emit_stamps_id_and_time_from_the_device_and_the_injected_clock() {
        let (bus, mut drain, clock) = bus(|c| c.first_seq = 41);
        let outcome = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        assert_eq!(outcome, Ok(EmitOutcome::Queued));
        clock.advance(1_000);
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        let mut sink = VecSink::default();
        let report = drain.pump(&mut sink).unwrap();
        assert_eq!(report.events, 2);
        let seqs: Vec<_> = sink.events.iter().map(|e| e.id.seq).collect();
        assert_eq!(seqs, [41, 42]);
        assert!(sink.events.iter().all(|e| e.id.device == device()));
        assert_eq!(sink.events[0].ts, UtcMillis::new(T0));
        assert_eq!(sink.events[1].ts, UtcMillis::new(T0 + 1_000));
    }

    #[test]
    fn a_disabled_kind_is_not_built_and_consumes_no_seq() {
        let (bus, mut drain, _) = bus(|_| {});
        bus.settings()
            .set_category(SignalCategory::SeekSkipReplay, false);
        let built = std::cell::Cell::new(0);
        let outcome = bus.emit_with(SignalKind::Paused, |s| {
            built.set(built.get() + 1);
            event(s, paused())
        });
        assert_eq!(outcome, Ok(EmitOutcome::Disabled));
        assert_eq!(built.get(), 0);
        let _ = bus.emit_with(SignalKind::ViewOpened, |s| event(s, view_opened()));
        let mut sink = VecSink::default();
        let _ = drain.pump(&mut sink).unwrap();
        assert_eq!(
            sink.events[0].id.seq, 0,
            "no seq burned by the disabled emit"
        );
    }

    #[test]
    fn counter_and_derived_kinds_cannot_be_emitted_as_events() {
        let (bus, _drain, _) = bus(|_| {});
        for kind in [
            SignalKind::Impression,
            SignalKind::PageDwell,
            SignalKind::ArtworkHover,
            SignalKind::PerceivedLevel,
            SignalKind::RateSession,
        ] {
            let r = bus.emit_with(kind, |_| unreachable!("must not build"));
            assert_eq!(r, Err(SignalsError::NotAnEvent(kind)));
        }
    }

    #[test]
    fn a_payload_of_another_kind_is_rejected() {
        let (bus, mut drain, _) = bus(|_| {});
        let r = bus.emit_with(SignalKind::ViewOpened, |s| event(s, paused()));
        assert_eq!(
            r,
            Err(SignalsError::KindMismatch {
                expected: SignalKind::ViewOpened,
                actual: SignalKind::Paused
            })
        );
        assert_eq!(drain.queued(), 0);
        let mut sink = VecSink::default();
        assert_eq!(drain.pump(&mut sink).unwrap().events, 0);
    }

    #[test]
    fn config_rejects_zero_capacities() {
        let clock = ManualClock::new(UtcMillis::new(T0));
        for tweak in [
            (|c: &mut BusConfig| c.reserved_capacity = 0) as fn(&mut BusConfig),
            |c| c.low_capacity = 0,
            |c| c.max_counter_keys = 0,
            |c| c.max_batch = 0,
            |c| c.derived_capacity = 0,
        ] {
            let mut cfg = BusConfig::new(device());
            tweak(&mut cfg);
            let r = SignalBus::channel(cfg, clock.clone(), Arc::new(SignalSettings::new()));
            assert!(matches!(r, Err(SignalsError::InvalidConfig { .. })));
        }
    }

    #[test]
    fn low_queue_evicts_the_oldest_and_counts_each_drop_by_kind() {
        let (bus, mut drain, _) = bus(|c| c.low_capacity = 3);
        for _ in 0..5 {
            let _ = bus.emit_with(SignalKind::ViewOpened, |s| event(s, view_opened()));
        }
        assert_eq!(bus.dropped(SignalKind::ViewOpened), 2);
        assert_eq!(bus.dropped_total(), 2);
        assert_eq!(bus.dropped_by_kind(), [(SignalKind::ViewOpened, 2)]);
        assert_eq!(bus.reserved_overflows(), 0);
        let mut sink = VecSink::default();
        let _ = drain.pump(&mut sink).unwrap();
        let seqs: Vec<_> = sink.events.iter().map(|e| e.id.seq).collect();
        assert_eq!(seqs, [2, 3, 4], "the oldest two were evicted");
    }

    #[test]
    fn reserved_events_are_never_evicted_by_a_low_priority_flood() {
        let (bus, mut drain, _) = bus(|c| {
            c.low_capacity = 4;
            c.reserved_capacity = 16;
        });
        let mut reserved_seqs = Vec::new();
        for i in 0..1_000 {
            let _ = bus.emit_with(SignalKind::ViewOpened, |s| event(s, view_opened()));
            if i % 100 == 0 {
                let mut seq = 0;
                let _ = bus.emit_with(SignalKind::Paused, |s| {
                    seq = s.id.seq;
                    event(s, paused())
                });
                reserved_seqs.push(seq);
            }
        }
        assert_eq!(bus.dropped(SignalKind::Paused), 0);
        assert_eq!(bus.reserved_overflows(), 0);
        let mut sink = VecSink::default();
        while drain.pump(&mut sink).unwrap().more {}
        let got: Vec<_> = sink
            .events
            .iter()
            .filter(|e| e.kind() == SignalKind::Paused)
            .map(|e| e.id.seq)
            .collect();
        assert_eq!(got, reserved_seqs);
        let low = sink
            .events
            .iter()
            .filter(|e| e.kind() == SignalKind::ViewOpened)
            .count() as u64;
        assert_eq!(low + bus.dropped(SignalKind::ViewOpened), 1_000);
    }

    #[test]
    fn a_full_reserved_queue_rejects_the_new_event_and_says_so() {
        let (bus, mut drain, _) = bus(|c| c.reserved_capacity = 2);
        let results: Vec<_> = (0..3)
            .map(|_| bus.emit_with(SignalKind::Paused, |s| event(s, paused())))
            .collect();
        assert_eq!(
            results,
            [
                Ok(EmitOutcome::Queued),
                Ok(EmitOutcome::Queued),
                Ok(EmitOutcome::Dropped)
            ]
        );
        assert_eq!(bus.reserved_overflows(), 1);
        assert_eq!(bus.dropped(SignalKind::Paused), 1);
        let mut sink = VecSink::default();
        let _ = drain.pump(&mut sink).unwrap();
        assert_eq!(sink.events.len(), 2, "the two accepted events are intact");
    }

    #[test]
    fn pump_orders_reserved_and_low_events_by_seq() {
        let (bus, mut drain, _) = bus(|_| {});
        let kinds = [
            (SignalKind::ViewOpened, view_opened()),
            (SignalKind::Paused, paused()),
            (SignalKind::ViewOpened, view_opened()),
            (SignalKind::Paused, paused()),
        ];
        for (kind, payload) in kinds {
            let _ = bus.emit_with(kind, |s| event(s, payload));
        }
        let mut sink = VecSink::default();
        let _ = drain.pump(&mut sink).unwrap();
        let seqs: Vec<_> = sink.events.iter().map(|e| e.id.seq).collect();
        assert_eq!(seqs, [0, 1, 2, 3]);
    }

    #[test]
    fn pump_batches_and_reports_when_more_is_waiting() {
        let (bus, mut drain, _) = bus(|c| c.max_batch = 3);
        for _ in 0..7 {
            let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        }
        let mut sink = VecSink::default();
        let r1 = drain.pump(&mut sink).unwrap();
        assert_eq!((r1.events, r1.more), (3, true));
        let r2 = drain.pump(&mut sink).unwrap();
        assert_eq!((r2.events, r2.more), (3, true));
        let r3 = drain.pump(&mut sink).unwrap();
        assert_eq!((r3.events, r3.more), (1, false));
        assert_eq!(sink.calls, 3);
        let r4 = drain.pump(&mut sink).unwrap();
        assert_eq!(r4.events, 0);
        assert_eq!(sink.calls, 3, "an empty pump does not call the sink");
    }

    #[test]
    fn a_failed_write_is_retried_with_the_same_batch() {
        let (bus, mut drain, _) = bus(|_| {});
        for _ in 0..3 {
            let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        }
        let _ = bus.counters().impression(track(5));
        let mut sink = VecSink {
            fail_next: true,
            ..VecSink::default()
        };
        let err = drain.pump(&mut sink).unwrap_err();
        assert_eq!(
            err,
            SignalsError::Sink {
                reason: "disk full".into()
            }
        );
        assert!(sink.events.is_empty());
        // A newer event and counter arrive while the sink is down.
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        let _ = bus.counters().impression(track(5));
        assert!(
            drain.wait(Duration::ZERO),
            "a retained batch counts as work"
        );
        let r = drain.pump(&mut sink).unwrap();
        assert_eq!((r.events, r.counter_rows), (3, 1));
        assert_eq!(sink.rows[0].count, 1, "the retained row, exactly once");
        let r = drain.pump(&mut sink).unwrap();
        assert_eq!((r.events, r.counter_rows), (1, 1));
        let seqs: Vec<_> = sink.events.iter().map(|e| e.id.seq).collect();
        assert_eq!(seqs, [0, 1, 2, 3], "no loss, no duplicate");
        assert_eq!(sink.rows.iter().map(|r| r.count).sum::<u64>(), 2);
    }

    #[test]
    fn wait_backs_off_for_the_timeout_while_a_failed_batch_is_retained() {
        const RETRY: Duration = Duration::from_millis(40);
        let (bus, mut drain, _) = bus(|_| {});
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        let mut sink = DeadSink::default();
        drain.pump(&mut sink).unwrap_err();
        // More events pile up behind the retained batch: the queues have work, so a `wait`
        // that only looked at the queues would return at once, forever.
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        let started = std::time::Instant::now();
        for attempt in 1..=4 {
            assert!(drain.wait(RETRY), "the retained batch is still work");
            drain.pump(&mut sink).unwrap_err();
            assert!(
                started.elapsed() >= RETRY * attempt,
                "attempt {attempt} came after only {:?}: the writer loop is spinning",
                started.elapsed()
            );
        }
        assert_eq!(
            sink.attempts, 5,
            "one failure, then one attempt per timeout"
        );
    }

    #[test]
    fn wait_backs_off_when_only_counter_rows_are_retained() {
        const RETRY: Duration = Duration::from_millis(40);
        let (bus, mut drain, _) = bus(|_| {});
        let _ = bus.counters().impression(track(1));
        let mut sink = DeadSink::default();
        drain.pump(&mut sink).unwrap_err();
        assert_eq!(
            drain.queued(),
            0,
            "no events: only counter rows are retained"
        );
        let started = std::time::Instant::now();
        assert!(drain.wait(RETRY));
        assert!(started.elapsed() >= RETRY);
    }

    #[test]
    fn wait_is_quick_again_once_the_retained_batch_is_written() {
        let (bus, mut drain, _) = bus(|_| {});
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        let mut sink = VecSink {
            fail_next: true,
            ..VecSink::default()
        };
        drain.pump(&mut sink).unwrap_err();
        assert!(drain.wait(Duration::from_millis(1)));
        drain.pump(&mut sink).unwrap();
        assert!(!drain.wait(Duration::from_millis(10)), "nothing left to do");
    }

    #[test]
    fn counters_flow_through_pump() {
        let (bus, mut drain, _) = bus(|_| {});
        for _ in 0..1_000 {
            let _ = bus.counters().impression(track(1));
        }
        let mut sink = VecSink::default();
        let r = drain.pump(&mut sink).unwrap();
        assert_eq!((r.events, r.counter_rows), (0, 1));
        assert_eq!(sink.rows[0].count, 1_000);
    }

    #[test]
    fn wait_returns_when_an_event_arrives_and_times_out_otherwise() {
        let (bus, drain, _) = bus(|_| {});
        assert!(!drain.wait(Duration::from_millis(10)));
        let producer = {
            let bus = bus.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
            })
        };
        assert!(drain.wait(Duration::from_secs(10)));
        producer.join().unwrap();
        assert_eq!(drain.queued(), 1);
    }

    #[test]
    fn the_drain_reports_closed_once_every_producer_is_gone() {
        let (bus, mut drain, _) = bus(|_| {});
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        let mut sink = VecSink::default();
        assert!(!drain.pump(&mut sink).unwrap().closed);
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
        drop(bus);
        let r = drain.pump(&mut sink).unwrap();
        assert_eq!(
            r.events, 1,
            "events queued before the drop are still delivered"
        );
        assert!(r.closed);
    }

    #[test]
    fn concurrent_producers_lose_no_reserved_event_and_account_for_every_low_one() {
        let (bus, mut drain, _) = bus(|c| {
            c.low_capacity = 64;
            c.reserved_capacity = 4_096;
        });
        let per_thread = 5_000_u64;
        let threads = 4_u64;
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let bus = bus.clone();
                std::thread::spawn(move || {
                    for i in 0..per_thread {
                        let _ = bus.emit_with(SignalKind::ViewOpened, |s| event(s, view_opened()));
                        if i % 10 == 0 {
                            let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
                        }
                    }
                })
            })
            .collect();
        let mut sink = VecSink::default();
        for h in handles {
            h.join().unwrap();
        }
        while drain.pump(&mut sink).unwrap().more {}
        let count = |k| sink.events.iter().filter(|e| e.kind() == k).count() as u64;
        assert_eq!(count(SignalKind::Paused), threads * per_thread / 10);
        assert_eq!(bus.dropped(SignalKind::Paused), 0);
        assert_eq!(bus.reserved_overflows(), 0);
        assert_eq!(
            count(SignalKind::ViewOpened) + bus.dropped(SignalKind::ViewOpened),
            threads * per_thread
        );
        let mut seqs: Vec<_> = sink.events.iter().map(|e| e.id.seq).collect();
        let before = seqs.len();
        seqs.sort_unstable();
        seqs.dedup();
        assert_eq!(seqs.len(), before, "seq is unique");
    }

    // --- derived values ---------------------------------------------------------------------

    fn rate_value(stamp: EventStamp, raw: u8) -> DerivedValue {
        DerivedValue {
            id: stamp.id,
            ts: stamp.ts,
            play: PlayRef {
                session: session(),
                recording: recording(3),
                started: UtcMillis::new(T0),
            },
            payload: DerivedPayload::RateSession(RateSessionPayload {
                rate: Rate::from_raw(raw),
            }),
        }
    }

    fn emit_rate(bus: &SignalBus, raw: u8) -> Result<EmitOutcome, SignalsError> {
        bus.emit_derived(SignalKind::RateSession, |s| rate_value(s, raw))
    }

    fn emit_pause(bus: &SignalBus) {
        let _ = bus.emit_with(SignalKind::Paused, |s| event(s, paused()));
    }

    fn rates(sink: &VecSink) -> Vec<u8> {
        sink.derived
            .iter()
            .map(|d| match d.payload {
                DerivedPayload::RateSession(p) => p.rate.raw(),
            })
            .collect()
    }

    #[test]
    fn a_derived_value_reaches_the_sink_in_seq_order_with_the_events() {
        let (bus, mut drain, _) = bus(|_| {});
        emit_pause(&bus);
        assert_eq!(emit_rate(&bus, 60), Ok(EmitOutcome::Queued));
        emit_pause(&bus);
        let mut sink = VecSink::default();
        let report = drain.pump(&mut sink).unwrap();
        assert_eq!((report.events, report.derived), (2, 1));
        let event_seqs: Vec<_> = sink.events.iter().map(|e| e.id.seq).collect();
        assert_eq!(event_seqs, [0, 2]);
        assert_eq!(sink.derived[0].id.seq, 1, "it spent a seq between the two");
        assert_eq!(sink.derived[0].id.device, device());
        assert_eq!(rates(&sink), [60]);
        assert_eq!(bus.queued_total(), 2, "a value is not an event");
        assert_eq!(bus.derived_queued_total(), 1);
        assert_eq!(drain.queued(), 0);
    }

    #[test]
    fn a_batch_cut_never_puts_a_value_before_an_event_emitted_before_it() {
        for (order, per_batch) in [
            // event, event, value: the value waits for the next batch.
            ([true, true, false], [(2, 0), (0, 1)]),
            // event, value, event: the value is in the first batch, the last event in the second.
            ([true, false, true], [(1, 1), (1, 0)]),
        ] {
            let (bus, mut drain, _) = bus(|c| c.max_batch = 2);
            for is_event in order {
                if is_event {
                    emit_pause(&bus);
                } else {
                    let _ = emit_rate(&bus, 50).unwrap();
                }
            }
            let mut sink = VecSink::default();
            for want in per_batch {
                let r = drain.pump(&mut sink).unwrap();
                assert_eq!((r.events, r.derived), want, "{order:?}");
            }
            assert_eq!(sink.events.len() + sink.derived.len(), 3);
        }
    }

    #[test]
    fn only_a_derived_kind_can_be_emitted_as_a_derived_value() {
        let (bus, mut drain, _) = bus(|_| {});
        for kind in [
            SignalKind::Paused,
            SignalKind::VolumeChanged,
            SignalKind::Impression,
            SignalKind::PageDwell,
        ] {
            let r = bus.emit_derived(kind, |_| unreachable!("must not build"));
            assert_eq!(r, Err(SignalsError::NotDerived(kind)));
        }
        assert_eq!(drain.queued(), 0);
        let mut sink = VecSink::default();
        assert_eq!(drain.pump(&mut sink).unwrap().derived, 0);
    }

    #[test]
    fn a_value_of_another_derived_kind_than_announced_is_rejected() {
        let (bus, mut drain, _) = bus(|_| {});
        let r = bus.emit_derived(SignalKind::PerceivedLevel, |s| rate_value(s, 50));
        assert_eq!(
            r,
            Err(SignalsError::KindMismatch {
                expected: SignalKind::PerceivedLevel,
                actual: SignalKind::RateSession
            })
        );
        assert_eq!(drain.queued(), 0);
        let mut sink = VecSink::default();
        assert_eq!(drain.pump(&mut sink).unwrap().derived, 0);
    }

    #[test]
    fn a_derived_kind_that_is_off_is_not_built_and_consumes_no_seq() {
        let (bus, mut drain, _) = bus(|_| {});
        // The category of its input, and the tier of its input.
        for turn_off in [
            (|b: &SignalBus| b.settings().set_category(SignalCategory::Speed, false))
                as fn(&SignalBus),
            |b| b.settings().set_tier(Tier::T0, false),
        ] {
            turn_off(&bus);
            let built = std::cell::Cell::new(0);
            let outcome = bus.emit_derived(SignalKind::RateSession, |s| {
                built.set(built.get() + 1);
                rate_value(s, 50)
            });
            assert_eq!(outcome, Ok(EmitOutcome::Disabled));
            assert_eq!(built.get(), 0);
            bus.settings().set_category(SignalCategory::Speed, true);
            bus.settings().set_tier(Tier::T0, true);
        }
        emit_pause(&bus);
        let mut sink = VecSink::default();
        let _ = drain.pump(&mut sink).unwrap();
        assert_eq!(sink.events[0].id.seq, 0, "no seq burned");
        assert_eq!(bus.derived_queued_total(), 0);
        assert_eq!(bus.derived_withdrawn(), 0);
    }

    #[test]
    fn a_derived_kind_follows_the_tier_of_its_inputs() {
        let (bus, _drain, _) = bus(|_| {});
        bus.settings().set_tier(Tier::T1, false);
        // `PerceivedLevel` takes the highest tier of its inputs (T1): off with T1 off.
        let r = bus.emit_derived(SignalKind::PerceivedLevel, |_| unreachable!("T1 is off"));
        assert_eq!(r, Ok(EmitOutcome::Disabled));
        assert!(bus.enabled(SignalKind::RateSession), "its inputs are T0");
    }

    #[test]
    fn a_value_queued_before_its_kind_was_switched_off_is_dropped_not_delivered() {
        let (bus, mut drain, _) = bus(|_| {});
        assert_eq!(emit_rate(&bus, 70), Ok(EmitOutcome::Queued));
        bus.settings().set_category(SignalCategory::Speed, false);
        let mut sink = VecSink::default();
        let r = drain.pump(&mut sink).unwrap();
        assert_eq!(r.derived, 0);
        assert!(sink.derived.is_empty());
        assert_eq!(bus.derived_withdrawn(), 1);
        // Collection that is back on delivers again.
        bus.settings().set_category(SignalCategory::Speed, true);
        assert_eq!(emit_rate(&bus, 71), Ok(EmitOutcome::Queued));
        let _ = drain.pump(&mut sink).unwrap();
        assert_eq!(rates(&sink), [71]);
        assert_eq!(bus.derived_withdrawn(), 1);
    }

    #[test]
    fn the_drain_can_say_whether_a_kind_is_still_collected() {
        let (bus, drain, _) = bus(|_| {});
        assert!(drain.enabled(SignalKind::RateSession));
        bus.settings().set_category(SignalCategory::Speed, false);
        assert!(!drain.enabled(SignalKind::RateSession));
    }

    #[test]
    fn a_full_derived_queue_loses_the_new_value_and_counts_it_under_its_kind() {
        let (bus, mut drain, _) = bus(|c| c.derived_capacity = 2);
        let results: Vec<_> = (0..3).map(|i| emit_rate(&bus, 50 + i)).collect();
        assert_eq!(
            results,
            [
                Ok(EmitOutcome::Queued),
                Ok(EmitOutcome::Queued),
                Ok(EmitOutcome::Dropped)
            ]
        );
        assert_eq!(bus.dropped(SignalKind::RateSession), 1);
        assert_eq!(bus.reserved_overflows(), 0, "not a playback event");
        let mut sink = VecSink::default();
        let _ = drain.pump(&mut sink).unwrap();
        assert_eq!(rates(&sink), [50, 51], "the accepted values are intact");
    }

    #[test]
    fn a_failed_write_is_retried_with_the_same_values() {
        let (bus, mut drain, _) = bus(|_| {});
        emit_pause(&bus);
        assert_eq!(emit_rate(&bus, 62), Ok(EmitOutcome::Queued));
        let mut sink = VecSink {
            fail_next: true,
            ..VecSink::default()
        };
        drain.pump(&mut sink).unwrap_err();
        assert!(sink.derived.is_empty());
        // A newer value arrives while the sink is down.
        assert_eq!(emit_rate(&bus, 63), Ok(EmitOutcome::Queued));
        assert!(drain.wait(Duration::ZERO), "a retained batch is work");
        let r = drain.pump(&mut sink).unwrap();
        assert_eq!((r.events, r.derived), (1, 1));
        let _ = drain.pump(&mut sink).unwrap();
        assert_eq!(rates(&sink), [62, 63], "no loss, no duplicate");
    }

    #[test]
    fn wait_returns_when_only_a_derived_value_arrives() {
        let (bus, drain, _) = bus(|_| {});
        let producer = {
            let bus = bus.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                let _ = emit_rate(&bus, 50);
            })
        };
        assert!(drain.wait(Duration::from_secs(10)));
        producer.join().unwrap();
        assert_eq!(drain.queued(), 1);
    }

    #[test]
    fn the_drain_is_not_closed_while_a_derived_value_is_waiting() {
        let (bus, mut drain, _) = bus(|_| {});
        assert_eq!(emit_rate(&bus, 50), Ok(EmitOutcome::Queued));
        drop(bus);
        let mut sink = VecSink::default();
        let r = drain.pump(&mut sink).unwrap();
        assert_eq!(r.derived, 1, "queued before the drop, still delivered");
        assert!(r.closed);
    }

    #[test]
    fn debug_output_never_prints_event_contents() {
        let (bus, drain, _) = bus(|_| {});
        let text = format!("{bus:?} {drain:?}");
        assert!(text.contains("SignalBus") && text.contains("SignalDrain"));
    }
}
