use serde::{Deserialize, Serialize};
use tinwave_core::{RecordingId, SessionId, SignalEventId, SourceId, Tier, UtcMillis};

use crate::codec::EncodedPayload;
use crate::{SignalKind, SignalPayload, SignalsError};

/// Id of a `context_snapshot` row (`signal_event.ctx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContextSnapshotId(pub u64);

/// One raw signal event, before storage.
///
/// Maps to a `signal_event` row in `docs/rules/database.md`:
///
/// | column | field |
/// |---|---|
/// | `id` | assigned by SQLite on insert; not part of the event |
/// | `device`, `seq` | `id` ([`SignalEventId`]); the writer maps the device UUID to its integer surrogate |
/// | `ts` | `ts` |
/// | `session` | `session`; the session row holds the device and timezone offset |
/// | `kind` | `payload.kind()` as its discriminant |
/// | `recording`, `source` | `recording`, `source`; the writer maps UUIDs to surrogates |
/// | `pos_ds` | `pos_ds` |
/// | `payload` | [`SignalEvent::encode_payload`]; `NULL` for kinds without fields |
/// | `ctx` | `ctx` |
///
/// The kind lives inside the payload enum, so an event can never carry a payload of another kind.
/// Tier is derived from the kind and never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalEvent {
    /// Device that recorded the event and its device-local sequence number.
    pub id: SignalEventId,
    /// When the event happened, UTC milliseconds.
    pub ts: UtcMillis,
    /// Session the event belongs to.
    pub session: SessionId,
    /// Recording the event is about, if any.
    pub recording: Option<RecordingId>,
    /// Source of that recording that was playing or acted on, if any.
    pub source: Option<SourceId>,
    /// Playback position in deciseconds, if the event has one.
    pub pos_ds: Option<u32>,
    /// The kind-specific payload; its variant is the event's kind.
    pub payload: SignalPayload,
    /// Context snapshot taken with the event, if context capture is on.
    pub ctx: Option<ContextSnapshotId>,
}

impl SignalEvent {
    /// The event's kind.
    #[must_use]
    pub const fn kind(&self) -> SignalKind {
        self.payload.kind()
    }

    /// The event's privacy tier, derived from its kind.
    #[must_use]
    pub const fn tier(&self) -> Tier {
        self.kind().tier()
    }

    /// The `payload` column value: `None` (SQL `NULL`) when the kind has no fields.
    ///
    /// # Errors
    /// See [`SignalPayload::encode`].
    pub fn encode_payload(&self) -> Result<Option<EncodedPayload>, SignalsError> {
        self.payload.encode()
    }
}

#[cfg(test)]
mod tests {
    use tinwave_core::DeviceId;

    use super::*;
    use crate::enums::RepeatMode;
    use crate::payload::RepeatModePayload;

    fn event(payload: SignalPayload) -> SignalEvent {
        let at = UtcMillis::new(1_700_000_000_000);
        SignalEvent {
            id: SignalEventId::new(DeviceId::from_parts(at, [1; 10]).unwrap(), 7),
            ts: at,
            session: SessionId::from_parts(at, [2; 10]).unwrap(),
            recording: Some(RecordingId::from_parts(at, [3; 10]).unwrap()),
            source: None,
            pos_ds: Some(1_234),
            payload,
            ctx: Some(ContextSnapshotId(3)),
        }
    }

    #[test]
    fn kind_and_tier_come_from_the_payload() {
        let e = event(SignalPayload::WindowState(
            crate::payload::WindowStatePayload {
                state: crate::enums::WindowStateKind::Tray,
            },
        ));
        assert_eq!(e.kind(), SignalKind::WindowState);
        assert_eq!(e.tier(), Tier::T1);
    }

    #[test]
    fn empty_payload_is_null_and_non_empty_is_bytes() {
        assert_eq!(event(SignalPayload::Favorited).encode_payload(), Ok(None));
        let e = event(SignalPayload::RepeatModeChanged(RepeatModePayload {
            mode: RepeatMode::One,
        }));
        let bytes = e.encode_payload().unwrap().unwrap();
        assert_eq!(bytes.as_bytes(), [1, 2]);
    }

    #[test]
    fn serde_round_trip() {
        let e = event(SignalPayload::Unfavorited);
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<SignalEvent>(&json).unwrap(), e);
    }
}
