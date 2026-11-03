//! Derived values: what the bus carries for the signals of storage class `D`.
//!
//! A derived kind (`RateSession`, later `PerceivedLevel`) is computed from raw events and never
//! stored as a `signal_event` row, so the bus refuses it as an event
//! ([`SignalsError::NotAnEvent`]). Its value still has to reach the writer, which stores it with
//! the play it describes. [`DerivedValue`] is that carrier, [`SignalBus::emit_derived`] the way in
//! and [`SignalSink::write_with_derived`] the way out (`docs/adr/0010-derived-values-on-the-bus.md`).
//!
//! # What a value says about its play
//!
//! A value is about **one play**, named by [`PlayRef`]: the session the play started in, the
//! recording, and the time of its `PlayStarted` event. That triple (with the device) is what the
//! writer's `play` row is made of, so the writer finds the row without a search and without
//! creating any dictionary or session row for a play that has none.
//!
//! # Order
//!
//! A value takes its `seq` from the same counter as the events and the drain hands out events and
//! values merged in `seq` order, a batch being a prefix of that order. Emit a value **just before**
//! the event that ends its play (`Completed` or `Skipped`): the two then reach the writer together,
//! and the writer, which commits at once on a play-end event, stores both in one transaction (the
//! record the outcome makes, then the value in it). A value emitted after its outcome works too,
//! in the same transaction or, at the latest, the writer's next flush, but then the outcome's flush
//! has already gone out without it.
//!
//! # Consent
//!
//! The bus checks [`SignalSettings::enabled`](crate::SignalSettings::enabled) when the value is
//! emitted and the drain checks it again when the value leaves the queue. A derived kind is
//! enabled only while its own category and tier **and the category and tier of every input**
//! (`SignalKind::derived_inputs`) are on, so a value computed from T1 inputs is not collected
//! with T1 off, nor one computed from volume events with Volume off.

use serde::{Deserialize, Serialize};
use tinwave_core::{RecordingId, SessionId, SignalEventId, Tier, UtcMillis};

use crate::{Rate, SignalKind};

/// The play a derived value describes: the identity of its `play` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayRef {
    /// The session the play started in (the session of its `PlayStarted` event).
    pub session: SessionId,
    /// The recording that was played.
    pub recording: RecordingId,
    /// The time of the play's `PlayStarted` event, UTC milliseconds, exactly as it was emitted.
    pub started: UtcMillis,
}

/// `RateSession`: the time-weighted playback rate of a play, quantized.
///
/// Source frames advanced per frame played over the whole play (1.0 is normal speed), rounded to
/// hundredths. Within 0.75 to 1.5 in practice; the type covers 0.50 to 3.05.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RateSessionPayload {
    /// The rate, as the `u8` of hundredths minus 50 that rates have everywhere else.
    pub rate: Rate,
}

/// The kind-specific part of a [`DerivedValue`]. One variant per derived kind that can be stored.
///
/// `PerceivedLevel` has no variant yet: it gets one together with the place the writer stores it
/// in (a `match` over this enum has no wildcard, so adding it makes every consumer decide).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DerivedPayload {
    /// The time-weighted rate of a play.
    RateSession(RateSessionPayload),
}

impl DerivedPayload {
    /// The derived kind this value is of.
    #[must_use]
    pub const fn kind(&self) -> SignalKind {
        match self {
            Self::RateSession(_) => SignalKind::RateSession,
        }
    }
}

/// One derived value, before storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedValue {
    /// Device and `seq`, taken from the bus counter at emit time. It orders the value among the
    /// events (module docs); it is never stored. Put it in unchanged from the bus's stamp.
    pub id: SignalEventId,
    /// When the value was derived (the end of the play), UTC milliseconds.
    pub ts: UtcMillis,
    /// The play it describes.
    pub play: PlayRef,
    /// The value; its variant is the kind.
    pub payload: DerivedPayload,
}

impl DerivedValue {
    /// The value's derived kind.
    #[must_use]
    pub const fn kind(&self) -> SignalKind {
        self.payload.kind()
    }

    /// The privacy tier of the kind: the highest tier among the inputs it is computed from.
    #[must_use]
    pub const fn tier(&self) -> Tier {
        self.kind().tier()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorageClass;

    #[test]
    fn every_payload_is_of_a_derived_kind_with_its_tier() {
        let payload = DerivedPayload::RateSession(RateSessionPayload { rate: Rate::NORMAL });
        assert_eq!(payload.kind(), SignalKind::RateSession);
        assert_eq!(payload.kind().storage_class(), StorageClass::Derived);
        assert_eq!(payload.kind().tier(), Tier::T0);
    }
}
