//! Tinwave signal schema: kinds, tiers, quantized payloads and the packed payload codec.
//!
//! This crate defines *what* a signal is and how its payload becomes a handful of bytes. It has
//! no IO, no bus and no storage; those live in later crates. The catalog in
//! `docs/guides/signal-catalog.md` is the source of truth, and `tests/catalog_sync.rs` fails
//! when this crate and the catalog disagree.
//!
//! - [`SignalKind`]: stable discriminants, [`StorageClass`] and privacy [`Tier`](tinwave_core::Tier).
//! - [`SignalPayload`]: one variant per raw-event kind, encoded to at most
//!   [`MAX_PAYLOAD_LEN`] bytes (`None` when a kind has no fields).
//! - [`SignalEvent`]: the in-memory shape of a `signal_event` row.

#![forbid(unsafe_code)]

mod codec;
mod enums;
mod error;
mod event;
mod kind;
mod payload;
mod quant;

pub use codec::{EncodedPayload, MAX_PAYLOAD_LEN};
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
