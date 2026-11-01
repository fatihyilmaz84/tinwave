//! Quantized value types used in payloads.
//!
//! Every type here trades precision for bytes (see `docs/rules/storage-budget.md` section 3).
//! Constructors from physical units quantize; accessors give the quantized value back in
//! physical units.

use serde::{Deserialize, Serialize};

use crate::SignalsError;

/// Rounds a unit-interval float to a per-255 byte.
fn unit_to_u8(value: f32, what: &'static str) -> Result<u8, SignalsError> {
    if !value.is_finite() {
        return Err(SignalsError::NotFinite { what });
    }
    // Clamped to 0..=255 first, so the cast cannot truncate or wrap.
    Ok((value.clamp(0.0, 1.0) * 255.0).round() as u8)
}

macro_rules! unit_u8 {
    ($(#[$meta:meta])* $name:ident, $what:literal, $from:ident, $to:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(u8);

        impl $name {
            /// Smallest value (0.0).
            pub const MIN: Self = Self(0);
            /// Largest value (1.0).
            pub const MAX: Self = Self(255);

            /// Wraps a raw per-255 value.
            #[must_use]
            pub const fn from_raw(raw: u8) -> Self {
                Self(raw)
            }

            /// The raw per-255 value.
            #[must_use]
            pub const fn raw(self) -> u8 {
                self.0
            }

            /// Quantizes a value in `0.0..=1.0`. Values outside the interval are clamped
            /// (slider and float noise); NaN and infinities are rejected.
            ///
            /// # Errors
            /// [`SignalsError::NotFinite`] for NaN or infinite input.
            pub fn $from(value: f32) -> Result<Self, SignalsError> {
                unit_to_u8(value, $what).map(Self)
            }

            /// The quantized value as a float in `0.0..=1.0`.
            #[must_use]
            pub fn $to(self) -> f32 {
                f32::from(self.0) / 255.0
            }
        }
    };
}

unit_u8! {
    /// Linear volume, `0.0..=1.0`, quantized to a `u8` (0-255).
    Volume, "volume", from_linear, linear
}

unit_u8! {
    /// A fraction of a track, `0.0..=1.0`, quantized to a `u8` (per-255).
    Fraction, "fraction", from_fraction, fraction
}

/// Playback rate quantized to a `u8` of hundredths minus 50, covering `0.50..=3.05`.
///
/// Tinwave offers 0.75x to 1.5x; the wider range leaves room for future presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rate(u8);

impl Rate {
    /// Normal speed (1.0x).
    pub const NORMAL: Self = Self(50);
    /// Slowest representable rate, 0.50x.
    pub const MIN_RATE: f32 = 0.50;
    /// Fastest representable rate, 3.05x.
    pub const MAX_RATE: f32 = 3.05;

    /// Wraps a raw `hundredths - 50` value.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    /// The raw `hundredths - 50` value.
    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Quantizes a playback rate (1.0 = normal) to hundredths.
    ///
    /// # Errors
    /// [`SignalsError::NotFinite`] for NaN/infinite input, [`SignalsError::InvalidValue`] when
    /// the rate is outside `0.50..=3.05` (a rate that far out is a bug, not a gesture).
    pub fn from_rate(rate: f32) -> Result<Self, SignalsError> {
        if !rate.is_finite() {
            return Err(SignalsError::NotFinite { what: "rate" });
        }
        let hundredths = (rate * 100.0).round();
        if !(50.0..=305.0).contains(&hundredths) {
            return Err(SignalsError::InvalidValue {
                what: "Rate",
                // Out of range, so the saturating cast only affects the diagnostic value.
                value: hundredths.clamp(0.0, 1_000_000.0) as u32,
            });
        }
        // In 50..=305, so the cast cannot truncate.
        Ok(Self((hundredths as u32 - 50) as u8))
    }

    /// The quantized rate (1.0 = normal).
    #[must_use]
    pub fn rate(self) -> f32 {
        (f32::from(self.0) + 50.0) / 100.0
    }
}

/// Duration of a gesture, quantized to 50 ms steps in a `u8` (saturates at 12.75 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GestureTime(u8);

impl GestureTime {
    /// Milliseconds per step.
    pub const STEP_MS: u32 = 50;

    /// Wraps a raw step count.
    #[must_use]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    /// The raw step count.
    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Quantizes milliseconds to the nearest step, saturating at the maximum.
    #[must_use]
    pub fn from_ms(ms: u32) -> Self {
        let steps = (u64::from(ms) + u64::from(Self::STEP_MS) / 2) / u64::from(Self::STEP_MS);
        Self(u8::try_from(steps).unwrap_or(u8::MAX))
    }

    /// The quantized duration in milliseconds.
    #[must_use]
    pub fn ms(self) -> u32 {
        u32::from(self.0) * Self::STEP_MS
    }
}

/// A short elapsed time in deciseconds, a fixed two-byte `u16` (saturates at about 109 minutes).
///
/// Used for "ms since track/section start" on volume gestures; the exact position is the
/// event's own `pos_ds` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElapsedDs(u16);

impl ElapsedDs {
    /// Wraps a raw decisecond count.
    #[must_use]
    pub const fn from_ds(ds: u16) -> Self {
        Self(ds)
    }

    /// The decisecond count.
    #[must_use]
    pub const fn ds(self) -> u16 {
        self.0
    }

    /// Quantizes milliseconds to the nearest decisecond, saturating at the maximum.
    #[must_use]
    pub fn from_ms(ms: u64) -> Self {
        Self(u16::try_from((ms.saturating_add(50)) / 100).unwrap_or(u16::MAX))
    }

    /// The quantized time in milliseconds.
    #[must_use]
    pub fn ms(self) -> u32 {
        u32::from(self.0) * 100
    }
}

/// A position or duration in deciseconds that fits a 3-byte varint: `0..=2_097_151`
/// (about 58 hours). Longer values saturate on construction from milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct PositionDs(u32);

impl PositionDs {
    /// Largest representable value, `2^21 - 1` deciseconds.
    pub const MAX: u32 = (1 << 21) - 1;
    /// Zero.
    pub const ZERO: Self = Self(0);

    /// Wraps a decisecond count.
    ///
    /// # Errors
    /// [`SignalsError::InvalidValue`] above [`Self::MAX`].
    pub const fn new(ds: u32) -> Result<Self, SignalsError> {
        if ds > Self::MAX {
            Err(SignalsError::InvalidValue {
                what: "PositionDs",
                value: ds,
            })
        } else {
            Ok(Self(ds))
        }
    }

    /// The decisecond count.
    #[must_use]
    pub const fn ds(self) -> u32 {
        self.0
    }

    /// Quantizes milliseconds to the nearest decisecond, saturating at [`Self::MAX`].
    #[must_use]
    pub fn from_ms(ms: u64) -> Self {
        let ds = ms.saturating_add(50) / 100;
        Self(u32::try_from(ds).map_or(Self::MAX, |ds| ds.min(Self::MAX)))
    }

    /// The quantized time in milliseconds.
    #[must_use]
    pub fn ms(self) -> u64 {
        u64::from(self.0) * 100
    }
}

impl TryFrom<u32> for PositionDs {
    type Error = SignalsError;

    fn try_from(ds: u32) -> Result<Self, Self::Error> {
        Self::new(ds)
    }
}

impl From<PositionDs> for u32 {
    fn from(value: PositionDs) -> Self {
        value.0
    }
}

/// A star rating, `0..=5`; `0` means the rating was cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Stars(u8);

impl Stars {
    /// Largest rating.
    pub const MAX: u8 = 5;

    /// Validates a star count.
    ///
    /// # Errors
    /// [`SignalsError::InvalidValue`] above 5.
    pub const fn new(stars: u8) -> Result<Self, SignalsError> {
        if stars > Self::MAX {
            Err(SignalsError::InvalidValue {
                what: "Stars",
                value: stars as u32,
            })
        } else {
            Ok(Self(stars))
        }
    }

    /// The star count.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Stars {
    type Error = SignalsError;

    fn try_from(stars: u8) -> Result<Self, Self::Error> {
        Self::new(stars)
    }
}

impl From<Stars> for u8 {
    fn from(value: Stars) -> Self {
        value.0
    }
}

/// A 32-bit hash stored as four fixed bytes (query hash, device hash).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hash32(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_quantizes_and_clamps() {
        assert_eq!(Volume::from_linear(0.0).unwrap(), Volume::MIN);
        assert_eq!(Volume::from_linear(1.0).unwrap(), Volume::MAX);
        assert_eq!(Volume::from_linear(0.5).unwrap().raw(), 128);
        assert_eq!(Volume::from_linear(-0.2).unwrap(), Volume::MIN);
        assert_eq!(Volume::from_linear(1.0001).unwrap(), Volume::MAX);
        assert!((Volume::from_raw(255).linear() - 1.0).abs() < f32::EPSILON);
        assert!(matches!(
            Volume::from_linear(f32::NAN),
            Err(SignalsError::NotFinite { .. })
        ));
        assert!(Fraction::from_fraction(f32::INFINITY).is_err());
    }

    #[test]
    fn volume_round_trips_within_half_a_step() {
        for raw in 0..=255u8 {
            let v = Volume::from_raw(raw);
            assert_eq!(Volume::from_linear(v.linear()).unwrap(), v);
        }
    }

    #[test]
    fn rate_is_hundredths_minus_fifty() {
        assert_eq!(Rate::from_rate(1.0).unwrap(), Rate::NORMAL);
        assert_eq!(Rate::from_rate(0.75).unwrap().raw(), 25);
        assert_eq!(Rate::from_rate(1.5).unwrap().raw(), 100);
        assert_eq!(Rate::from_rate(0.5).unwrap().raw(), 0);
        assert_eq!(Rate::from_rate(3.05).unwrap().raw(), 255);
        assert!((Rate::from_raw(25).rate() - 0.75).abs() < 1e-6);
        assert!(matches!(
            Rate::from_rate(0.49),
            Err(SignalsError::InvalidValue { what: "Rate", .. })
        ));
        assert!(Rate::from_rate(3.1).is_err());
        assert!(Rate::from_rate(f32::NAN).is_err());
    }

    #[test]
    fn gesture_time_rounds_to_50ms_and_saturates() {
        assert_eq!(GestureTime::from_ms(0).raw(), 0);
        assert_eq!(GestureTime::from_ms(24).raw(), 0);
        assert_eq!(GestureTime::from_ms(25).raw(), 1);
        assert_eq!(GestureTime::from_ms(1_000).raw(), 20);
        assert_eq!(GestureTime::from_ms(u32::MAX), GestureTime::from_raw(255));
        assert_eq!(GestureTime::from_raw(20).ms(), 1_000);
    }

    #[test]
    fn elapsed_ds_saturates() {
        assert_eq!(ElapsedDs::from_ms(15_000).ds(), 150);
        assert_eq!(ElapsedDs::from_ms(u64::MAX), ElapsedDs::from_ds(u16::MAX));
        assert_eq!(ElapsedDs::from_ds(150).ms(), 15_000);
    }

    #[test]
    fn position_ds_validates_and_saturates() {
        assert_eq!(PositionDs::new(PositionDs::MAX).unwrap().ds(), 2_097_151);
        assert_eq!(
            PositionDs::new(PositionDs::MAX + 1),
            Err(SignalsError::InvalidValue {
                what: "PositionDs",
                value: PositionDs::MAX + 1
            })
        );
        assert_eq!(PositionDs::from_ms(240_000).ds(), 2_400);
        assert_eq!(PositionDs::from_ms(u64::MAX).ds(), PositionDs::MAX);
        assert_eq!(PositionDs::from_ms(49), PositionDs::ZERO);
        assert_eq!(PositionDs::from_ms(50).ds(), 1);
        assert!(serde_json::from_str::<PositionDs>("2097152").is_err());
        assert_eq!(serde_json::from_str::<PositionDs>("7").unwrap().ds(), 7);
    }

    #[test]
    fn stars_range() {
        assert_eq!(Stars::new(5).unwrap().get(), 5);
        assert!(Stars::new(6).is_err());
        assert!(serde_json::from_str::<Stars>("9").is_err());
    }
}
