//! Tinwave signals: the schema, the packed payload codec and the in-memory signal bus.
//!
//! This crate defines *what* a signal is, how its payload becomes a handful of bytes, and how
//! signals travel from producers to the writer without ever blocking them. It does no IO and
//! knows no database: the writer (a later crate) implements [`SignalSink`]. The catalog in
//! `docs/guides/signal-catalog.md` is the source of truth, and `tests/catalog_sync.rs` fails
//! when this crate and the catalog disagree.
//!
//! - [`SignalKind`]: stable discriminants, [`StorageClass`] and privacy [`Tier`](tinwave_core::Tier).
//! - [`SignalPayload`]: one variant per raw-event kind, encoded to at most
//!   [`MAX_PAYLOAD_LEN`] bytes (`None` when a kind has no fields).
//! - [`SignalEvent`]: the in-memory shape of a `signal_event` row.
//! - [`SignalBus`] / [`SignalDrain`] / [`SignalSink`]: bounded, non-blocking transport with a
//!   priority drop policy ([`Priority`]); [`SignalSettings`] toggles are checked before an event
//!   is constructed.
//! - [`UiCounters`]: in-memory aggregation of the counter-class signals.
//! - [`GestureCoalescer`]: one event per volume / rate / system-volume gesture and per burst of
//!   seeks.

#![forbid(unsafe_code)]

mod bus;
mod coalesce;
mod codec;
mod context_key;
mod counters;
mod enums;
mod error;
mod event;
#[cfg(test)]
mod frozen;
mod kind;
mod payload;
mod priority;
mod quant;
mod settings;
#[cfg(test)]
mod test_support;

pub use bus::{BusConfig, EmitOutcome, EventStamp, PumpReport, SignalBus, SignalDrain, SignalSink};
pub use coalesce::{
    coalesce_window_ms, EventContext, GestureCoalescer, GestureTick, TickOutcome, RATE_WINDOW_MS,
    SEEK_MIN_TOTAL_DS, SEEK_WINDOW_MS, SYSTEM_VOLUME_WINDOW_MS, VOLUME_WINDOW_MS,
};
pub use counters::{CountOutcome, CounterEntity, CounterRow, LocalDay, UiCounters};
pub use priority::Priority;
pub use settings::{SignalCategory, SignalSettings};

pub use codec::{EncodedPayload, MAX_PAYLOAD_LEN};
pub use context_key::{
    ActivityGuess, ContextKey, Daypart, PlaceSlot, TemperatureBand, WeatherCondition, WeekdayType,
};
pub use enums::{
    ContextScope, EntityKind, InputSource, MetadataFields, OutputClass, OutputTransport,
    PlayOrigin, PlaylistKind, QueueOrigin, RadioFeedbackKind, RepeatMode, SearchScope, Section,
    SectionKind, View, WindowStateKind,
};
pub use error::SignalsError;
pub use event::{ContextSnapshotId, SignalEvent};
pub use kind::{SignalKind, StorageClass};
pub use payload::{
    CompletedPayload, CrossfadePayload, EqPayload, ExclusiveModePayload, ExplanationOpenedPayload,
    FieldSpec, MetadataEditedPayload, MutePayload, OutputDevicePayload, PausedPayload,
    PlayStartedPayload, PlaylistEditPayload, QueueReorderedPayload, QueuedPayload,
    RadioFeedbackPayload, RatePayload, RatedPayload, RemovedFromQueuePayload, RepeatModePayload,
    ReplayedPayload, ResumedPayload, SearchNoResultPayload, SearchPerformedPayload,
    SearchResultOpenedPayload, SeekPayload, SessionEndedPayload, SessionStartedPayload,
    SignalPayload, SkipPayload, SystemVolumePayload, ViewOpenedPayload, VolumePayload,
    WindowStatePayload,
};
pub use quant::{ElapsedDs, Fraction, GestureTime, Hash32, PositionDs, Rate, Stars, Volume};
