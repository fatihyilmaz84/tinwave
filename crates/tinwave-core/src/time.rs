use std::fmt;

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// A point in time as signed milliseconds since the Unix epoch, UTC.
///
/// Storage never holds naive local time; pair with [`TzOffsetMin`] when the local
/// wall-clock matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcMillis(pub i64);

impl UtcMillis {
    /// Wraps a raw millisecond count.
    #[must_use]
    pub const fn new(ms: i64) -> Self {
        Self(ms)
    }

    /// The raw millisecond count.
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self.0
    }
}

impl fmt::Display for UtcMillis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

/// Offset of local time from UTC in minutes (east positive), valid range -720..=840.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "i16", into = "i16")]
pub struct TzOffsetMin(i16);

impl TzOffsetMin {
    /// Smallest real-world offset, UTC-12:00.
    pub const MIN: i16 = -12 * 60;
    /// Largest real-world offset, UTC+14:00.
    pub const MAX: i16 = 14 * 60;
    /// UTC itself.
    pub const UTC: Self = Self(0);

    /// Validates and wraps an offset in minutes.
    ///
    /// # Errors
    /// [`CoreError::TzOffsetOutOfRange`] when outside -720..=840.
    pub const fn new(minutes: i16) -> Result<Self, CoreError> {
        if minutes < Self::MIN || minutes > Self::MAX {
            Err(CoreError::TzOffsetOutOfRange(minutes))
        } else {
            Ok(Self(minutes))
        }
    }

    /// The offset in minutes.
    #[must_use]
    pub const fn minutes(self) -> i16 {
        self.0
    }
}

impl TryFrom<i16> for TzOffsetMin {
    type Error = CoreError;

    fn try_from(minutes: i16) -> Result<Self, Self::Error> {
        Self::new(minutes)
    }
}

impl From<TzOffsetMin> for i16 {
    fn from(value: TzOffsetMin) -> Self {
        value.0
    }
}

impl fmt::Display for TzOffsetMin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { '-' } else { '+' };
        let abs = self.0.unsigned_abs();
        write!(f, "UTC{sign}{:02}:{:02}", abs / 60, abs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_millis_orders_and_round_trips() {
        assert!(UtcMillis::new(1) < UtcMillis::new(2));
        let t = UtcMillis::new(-5);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "-5");
        assert_eq!(serde_json::from_str::<UtcMillis>(&json).unwrap(), t);
        assert_eq!(t.to_string(), "-5ms");
    }

    #[test]
    fn tz_offset_validates_range() {
        assert_eq!(TzOffsetMin::new(0).unwrap(), TzOffsetMin::UTC);
        assert_eq!(TzOffsetMin::new(-720).unwrap().minutes(), -720);
        assert_eq!(TzOffsetMin::new(840).unwrap().minutes(), 840);
        assert_eq!(
            TzOffsetMin::new(841),
            Err(CoreError::TzOffsetOutOfRange(841))
        );
        assert_eq!(
            TzOffsetMin::new(-721),
            Err(CoreError::TzOffsetOutOfRange(-721))
        );
    }

    #[test]
    fn tz_offset_serde_round_trip_and_rejects_invalid() {
        let tz = TzOffsetMin::new(330).unwrap();
        let json = serde_json::to_string(&tz).unwrap();
        assert_eq!(json, "330");
        assert_eq!(serde_json::from_str::<TzOffsetMin>(&json).unwrap(), tz);
        assert!(serde_json::from_str::<TzOffsetMin>("5000").is_err());
    }

    #[test]
    fn tz_offset_display() {
        assert_eq!(TzOffsetMin::new(330).unwrap().to_string(), "UTC+05:30");
        assert_eq!(TzOffsetMin::new(-480).unwrap().to_string(), "UTC-08:00");
        assert_eq!(TzOffsetMin::UTC.to_string(), "UTC+00:00");
    }
}
