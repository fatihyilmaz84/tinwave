use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::{NoContext, Timestamp, Uuid};

use crate::{CoreError, UtcMillis};

/// Largest millisecond value a UUIDv7 can carry (48 bits).
const MAX_V7_MILLIS: i64 = (1 << 48) - 1;

/// Defines a UUIDv7-backed newtype id.
///
/// Ids are time-ordered: sorting by id sorts by creation time (to the millisecond).
/// Parsing and deserializing reject UUIDs that are not version 7.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new id stamped with the current system time.
            ///
            /// Ids generated within one process are monotonic. Domain code that must be
            /// deterministic under test should use [`Self::new_at`] with an injected clock.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Generates a new id stamped with `at`; the random part is fresh.
            ///
            /// Two ids created for the same millisecond are not ordered relative to each other.
            ///
            /// # Errors
            /// [`CoreError::TimestampOutOfRange`] if `at` is before 1970 or beyond 48 bits.
            pub fn new_at(at: UtcMillis) -> Result<Self, CoreError> {
                let ms = at.as_i64();
                if !(0..=MAX_V7_MILLIS).contains(&ms) {
                    return Err(CoreError::TimestampOutOfRange(ms));
                }
                // Non-negative by the range check above.
                let ms = ms.unsigned_abs();
                let secs = ms / 1000;
                // (ms % 1000) * 1_000_000 < 1_000_000_000, so it always fits in u32.
                let nanos = u32::try_from((ms % 1000) * 1_000_000)
                    .map_err(|_| CoreError::TimestampOutOfRange(at.as_i64()))?;
                Ok(Self(Uuid::new_v7(Timestamp::from_unix(NoContext, secs, nanos))))
            }

            /// Wraps an existing UUID.
            ///
            /// # Errors
            /// [`CoreError::NotUuidV7`] if `uuid` is not version 7.
            pub fn from_uuid(uuid: Uuid) -> Result<Self, CoreError> {
                if uuid.get_version_num() == 7 {
                    Ok(Self(uuid))
                } else {
                    Err(CoreError::NotUuidV7(uuid))
                }
            }

            /// The underlying UUID.
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// The 16 raw bytes, for compact storage.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }

            /// Wraps 16 raw bytes read from storage.
            ///
            /// # Errors
            /// [`CoreError::NotUuidV7`] if the bytes are not a UUIDv7.
            pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, CoreError> {
                Self::from_uuid(Uuid::from_bytes(bytes))
            }

            /// The creation time embedded in the id, to the millisecond.
            #[must_use]
            pub fn created_at(&self) -> UtcMillis {
                let b = self.0.as_bytes();
                let ms = u64::from_be_bytes([0, 0, b[0], b[1], b[2], b[3], b[4], b[5]]);
                // 48 bits always fit in i64.
                UtcMillis::new(i64::try_from(ms).unwrap_or(MAX_V7_MILLIS))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self.0.hyphenated())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0.hyphenated(), f)
            }
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let uuid = Uuid::parse_str(s)
                    .map_err(|_| CoreError::InvalidIdText(s.to_owned()))?;
                Self::from_uuid(uuid)
            }
        }

        impl TryFrom<Uuid> for $name {
            type Error = CoreError;

            fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
                Self::from_uuid(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

uuid_id! {
    /// Identity of a musical recording, independent of any file or server item.
    RecordingId
}

uuid_id! {
    /// Identity of one playable source of a recording (local file, server item, downloaded copy).
    SourceId
}

uuid_id! {
    /// Identity of a listening or radio session.
    SessionId
}

uuid_id! {
    /// Identity of one Tinwave installation / device.
    DeviceId
}

/// Identity of a signal event: the device that recorded it and that device's monotonic sequence.
///
/// Orders by `(device, seq)`. This is the dedupe key for sync; it replaces a per-event UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SignalEventId {
    /// Device that recorded the event.
    pub device: DeviceId,
    /// Device-local sequence number, strictly increasing per device.
    pub seq: u64,
}

impl SignalEventId {
    /// Builds an event id.
    #[must_use]
    pub const fn new(device: DeviceId, seq: u64) -> Self {
        Self { device, seq }
    }
}

impl fmt::Display for SignalEventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.device, self.seq)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::thread::sleep;
    use std::time::Duration;

    use super::*;

    fn assert_id_traits<T>()
    where
        T: Copy + Eq + Ord + std::hash::Hash + fmt::Debug + fmt::Display,
    {
    }

    #[test]
    fn all_ids_implement_required_traits() {
        assert_id_traits::<RecordingId>();
        assert_id_traits::<SourceId>();
        assert_id_traits::<SessionId>();
        assert_id_traits::<DeviceId>();
        assert_id_traits::<SignalEventId>();
    }

    #[test]
    fn generated_ids_are_uuid_v7() {
        let id = RecordingId::new();
        assert_eq!(id.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn ids_generated_over_time_sort_in_creation_order() {
        let mut ids = Vec::new();
        for _ in 0..5 {
            ids.push(RecordingId::new());
            sleep(Duration::from_millis(3));
        }
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn ids_in_a_tight_loop_are_unique_and_monotonic() {
        let ids: Vec<SourceId> = (0..1000).map(|_| SourceId::new()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
    }

    #[test]
    fn new_at_embeds_and_orders_by_the_given_time() {
        let early = SessionId::new_at(UtcMillis::new(1_700_000_000_000)).unwrap();
        let late = SessionId::new_at(UtcMillis::new(1_700_000_000_001)).unwrap();
        assert_eq!(early.created_at(), UtcMillis::new(1_700_000_000_000));
        assert_eq!(late.created_at(), UtcMillis::new(1_700_000_000_001));
        assert!(early < late);
        assert_eq!(early.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn new_at_handles_epoch_and_max_and_rejects_out_of_range() {
        assert_eq!(
            DeviceId::new_at(UtcMillis::new(0)).unwrap().created_at(),
            UtcMillis::new(0)
        );
        let max = UtcMillis::new((1 << 48) - 1);
        assert_eq!(DeviceId::new_at(max).unwrap().created_at(), max);
        assert_eq!(
            DeviceId::new_at(UtcMillis::new(-1)),
            Err(CoreError::TimestampOutOfRange(-1))
        );
        assert_eq!(
            DeviceId::new_at(UtcMillis::new(1 << 48)),
            Err(CoreError::TimestampOutOfRange(1 << 48))
        );
    }

    #[test]
    fn created_at_tracks_the_system_clock() {
        let id = RecordingId::new();
        let ms = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        assert!((ms - id.created_at().as_i64()).abs() < 5_000);
    }

    #[test]
    fn display_and_parse_round_trip() {
        let id = RecordingId::new();
        let text = id.to_string();
        assert_eq!(text.len(), 36);
        assert_eq!(text.parse::<RecordingId>().unwrap(), id);
        assert!(format!("{id:?}").starts_with("RecordingId("));
    }

    #[test]
    fn parse_rejects_garbage_and_non_v7() {
        assert!(matches!(
            "nope".parse::<SourceId>(),
            Err(CoreError::InvalidIdText(_))
        ));
        let v4 = "67e55044-10b1-426f-9247-bb680e5fe0c8";
        assert!(matches!(
            v4.parse::<SourceId>(),
            Err(CoreError::NotUuidV7(_))
        ));
        assert!(serde_json::from_str::<SourceId>(&format!("\"{v4}\"")).is_err());
    }

    #[test]
    fn bytes_round_trip() {
        let id = DeviceId::new();
        assert_eq!(DeviceId::from_bytes(*id.as_bytes()).unwrap(), id);
        assert!(DeviceId::from_bytes([0; 16]).is_err());
    }

    #[test]
    fn uuid_ids_serde_round_trip_as_plain_strings() {
        let id = RecordingId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<RecordingId>(&json).unwrap(), id);

        let id = SessionId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), id);

        let id = SourceId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<SourceId>(&json).unwrap(), id);

        let id = DeviceId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<DeviceId>(&json).unwrap(), id);
    }

    #[test]
    fn signal_event_id_orders_by_device_then_seq() {
        let d1 = DeviceId::new_at(UtcMillis::new(1_000)).unwrap();
        let d2 = DeviceId::new_at(UtcMillis::new(2_000)).unwrap();
        assert!(d1 < d2);

        let a = SignalEventId::new(d1, 900);
        let b = SignalEventId::new(d1, 901);
        let c = SignalEventId::new(d2, 1);
        assert!(a < b, "same device orders by seq");
        assert!(b < c, "device dominates seq");
        assert!(a < c);

        let mut shuffled = vec![c, a, b];
        shuffled.sort();
        assert_eq!(shuffled, vec![a, b, c]);
    }

    #[test]
    fn signal_event_id_serde_round_trip_and_display() {
        let id = SignalEventId::new(DeviceId::new(), u64::MAX);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<SignalEventId>(&json).unwrap(), id);
        assert_eq!(id.to_string(), format!("{}#{}", id.device, u64::MAX));
    }
}
