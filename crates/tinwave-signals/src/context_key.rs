//! `ContextKey`: the taste engine's context bucket, packed into 32 bits.
//!
//! A play record stores this instead of a full context snapshot (`docs/rules/storage-budget.md`,
//! section 4). The layout is versioned and persisted in `play.context_key` (and
//! `context_snapshot.key`), so it is frozen: a field never moves, a code is never reused, and a
//! change needs a new layout version. The layout is documented for readers of the data in
//! `docs/guides/taste-engine.md` (section "ContextKey layout").
//!
//! # Layout, version 1
//!
//! Bit 0 is the least significant bit. **Every field's value 0 means "unknown"**; the known
//! values start at 1.
//!
//! | bits | width | field | known values |
//! |---|---|---|---|
//! | 0-2 | 3 | daypart | 1..=7 [`Daypart`] |
//! | 3-4 | 2 | weekday type | 1..=3 [`WeekdayType`] |
//! | 5-7 | 3 | weather condition | 1..=6 [`WeatherCondition`] |
//! | 8-10 | 3 | temperature band | 1..=5 [`TemperatureBand`] |
//! | 11-13 | 3 | device class | 1..=7: the [`OutputClass`] codes 0..=6 plus one; `OutputClass::Unknown` is 0 |
//! | 14-17 | 4 | activity | 1..=9 [`ActivityGuess`] |
//! | 18-23 | 6 | place slot | 1..=63 [`PlaceSlot`] |
//! | 24-27 | 4 | reserved | always 0 |
//! | 28-31 | 4 | layout version | 1 |
//!
//! # The all-unknown key is 0
//!
//! A key with no known field is the raw value `0`, and so is the value stored for a play whose
//! context was never captured: "nothing known" is one value, not a guess. Because version 0 is
//! then only ever the all-unknown key, [`ContextKey::raw`] writes the layout version only when at
//! least one field is known, and [`ContextKey::from_raw`] accepts `0` and version-1 values
//! (a version-1 value with every field 0 reads as all-unknown). Any other version, a non-zero
//! reserved nibble or a field code with no meaning is an error, never a silent clamp.

use crate::{OutputClass, SignalsError};

/// Defines a field enum with explicit, frozen codes starting at 1 (0 is "unknown").
macro_rules! field_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $($(#[$vmeta:meta])* $variant:ident = $code:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
        }

        impl $name {
            /// All variants, in code order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The code stored in the key, `1..`; 0 is reserved for "unknown".
            #[must_use]
            pub const fn code(self) -> u8 {
                match self { $(Self::$variant => $code,)* }
            }

            /// The variant for a stored code; `None` for a code with no meaning (0 included).
            #[must_use]
            pub const fn from_code(code: u8) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

field_enum! {
    /// Part of the day, from the clock provider (`docs/guides/signal-catalog.md` section 7).
    Daypart {
        Night = 1,
        EarlyMorning = 2,
        Morning = 3,
        Midday = 4,
        Afternoon = 5,
        Evening = 6,
        LateNight = 7,
    }
}

field_enum! {
    /// Kind of day (`docs/guides/taste-engine.md`: `weekday_type(Weekday|Weekend|Holiday)`).
    WeekdayType {
        Weekday = 1,
        Weekend = 2,
        Holiday = 3,
    }
}

field_enum! {
    /// Weather condition of a `WeatherBucket` (`docs/guides/context-engine.md`).
    WeatherCondition {
        Clear = 1,
        Cloudy = 2,
        Rain = 3,
        Storm = 4,
        Snow = 5,
        Fog = 6,
    }
}

field_enum! {
    /// Temperature band of a `WeatherBucket` (`docs/guides/context-engine.md`):
    /// below 5, 5 to 15, 15 to 24, 24 to 30 and above 30 degrees Celsius.
    TemperatureBand {
        Cold = 1,
        Cool = 2,
        Mild = 3,
        Warm = 4,
        Hot = 5,
    }
}

field_enum! {
    /// The context engine's deterministic activity guess (`docs/guides/context-engine.md`,
    /// "Derived context"), including the mobile values.
    ActivityGuess {
        Focus = 1,
        Gaming = 2,
        Call = 3,
        Background = 4,
        OnTheMove = 5,
        Leisure = 6,
        Workout = 7,
        Commute = 8,
        Driving = 9,
    }
}

/// The slot of a network place in the device's own small place table (`1..=63`). A slot is an
/// index, never an SSID, a MAC or a hash of one: the same place has different slots on different
/// devices, and a key is meaningful per device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlaceSlot(u8);

impl PlaceSlot {
    /// Largest slot (6 bits, with 0 meaning "unknown").
    pub const MAX: u8 = 63;

    /// Wraps a slot.
    ///
    /// # Errors
    /// [`SignalsError::InvalidValue`] for 0 (that is "unknown": leave the field unset) or above 63.
    pub const fn new(slot: u8) -> Result<Self, SignalsError> {
        if slot == 0 || slot > Self::MAX {
            Err(SignalsError::InvalidValue {
                what: "PlaceSlot",
                value: slot as u32,
            })
        } else {
            Ok(Self(slot))
        }
    }

    /// The slot, `1..=63`.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// The device class field of the key: an [`OutputClass`] plus one, with `Unknown` as 0.
const fn device_code(class: OutputClass) -> u32 {
    match class {
        OutputClass::Headphones => 1,
        OutputClass::Speakers => 2,
        OutputClass::Bluetooth => 3,
        OutputClass::UsbDac => 4,
        OutputClass::Hdmi => 5,
        OutputClass::Builtin => 6,
        OutputClass::Virtual => 7,
        // "Unknown" is the unknown value of the field.
        OutputClass::Unknown => 0,
    }
}

/// The class a device field value stands for; `None` for 0 and for `OutputClass::Unknown`.
const fn device_from_code(code: u32) -> Option<OutputClass> {
    match code {
        1 => Some(OutputClass::Headphones),
        2 => Some(OutputClass::Speakers),
        3 => Some(OutputClass::Bluetooth),
        4 => Some(OutputClass::UsbDac),
        5 => Some(OutputClass::Hdmi),
        6 => Some(OutputClass::Builtin),
        7 => Some(OutputClass::Virtual),
        _ => None,
    }
}

const DAYPART_SHIFT: u32 = 0;
const WEEKDAY_SHIFT: u32 = 3;
const CONDITION_SHIFT: u32 = 5;
const TEMPERATURE_SHIFT: u32 = 8;
const DEVICE_SHIFT: u32 = 11;
const ACTIVITY_SHIFT: u32 = 14;
const PLACE_SHIFT: u32 = 18;
const RESERVED_SHIFT: u32 = 24;
const VERSION_SHIFT: u32 = 28;

const DAYPART_MASK: u32 = 0b111;
const WEEKDAY_MASK: u32 = 0b11;
const CONDITION_MASK: u32 = 0b111;
const TEMPERATURE_MASK: u32 = 0b111;
const DEVICE_MASK: u32 = 0b111;
const ACTIVITY_MASK: u32 = 0b1111;
const PLACE_MASK: u32 = 0b11_1111;
const RESERVED_MASK: u32 = 0b1111;
const VERSION_MASK: u32 = 0b1111;

/// The context a play happened in, as the taste engine buckets it. Every field may be unknown.
///
/// See the module docs for the bit layout. Build one with the `with_*` methods starting from
/// [`ContextKey::UNKNOWN`]; [`raw`](Self::raw) packs it and [`from_raw`](Self::from_raw) unpacks
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContextKey {
    daypart: Option<Daypart>,
    weekday_type: Option<WeekdayType>,
    condition: Option<WeatherCondition>,
    temperature: Option<TemperatureBand>,
    device_class: Option<OutputClass>,
    activity: Option<ActivityGuess>,
    place: Option<PlaceSlot>,
}

impl ContextKey {
    /// The layout version [`raw`](Self::raw) writes.
    pub const LAYOUT_VERSION: u8 = 1;

    /// Nothing known. Packs to the raw value 0.
    pub const UNKNOWN: Self = Self {
        daypart: None,
        weekday_type: None,
        condition: None,
        temperature: None,
        device_class: None,
        activity: None,
        place: None,
    };

    /// This key with the daypart set.
    #[must_use]
    pub const fn with_daypart(mut self, daypart: Daypart) -> Self {
        self.daypart = Some(daypart);
        self
    }

    /// This key with the kind of day set.
    #[must_use]
    pub const fn with_weekday_type(mut self, weekday_type: WeekdayType) -> Self {
        self.weekday_type = Some(weekday_type);
        self
    }

    /// This key with the weather condition set.
    #[must_use]
    pub const fn with_condition(mut self, condition: WeatherCondition) -> Self {
        self.condition = Some(condition);
        self
    }

    /// This key with the temperature band set.
    #[must_use]
    pub const fn with_temperature(mut self, temperature: TemperatureBand) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// This key with the output device class set. `OutputClass::Unknown` leaves the field
    /// unknown: the two are the same value.
    #[must_use]
    pub const fn with_device_class(mut self, class: OutputClass) -> Self {
        self.device_class = match class {
            OutputClass::Unknown => None,
            known => Some(known),
        };
        self
    }

    /// This key with the activity guess set.
    #[must_use]
    pub const fn with_activity(mut self, activity: ActivityGuess) -> Self {
        self.activity = Some(activity);
        self
    }

    /// This key with the place slot set.
    #[must_use]
    pub const fn with_place(mut self, place: PlaceSlot) -> Self {
        self.place = Some(place);
        self
    }

    /// The daypart, if known.
    #[must_use]
    pub const fn daypart(self) -> Option<Daypart> {
        self.daypart
    }

    /// The kind of day, if known.
    #[must_use]
    pub const fn weekday_type(self) -> Option<WeekdayType> {
        self.weekday_type
    }

    /// The weather condition, if known.
    #[must_use]
    pub const fn condition(self) -> Option<WeatherCondition> {
        self.condition
    }

    /// The temperature band, if known.
    #[must_use]
    pub const fn temperature(self) -> Option<TemperatureBand> {
        self.temperature
    }

    /// The output device class, if known (never `OutputClass::Unknown`).
    #[must_use]
    pub const fn device_class(self) -> Option<OutputClass> {
        self.device_class
    }

    /// The activity guess, if known.
    #[must_use]
    pub const fn activity(self) -> Option<ActivityGuess> {
        self.activity
    }

    /// The place slot, if known.
    #[must_use]
    pub const fn place(self) -> Option<PlaceSlot> {
        self.place
    }

    /// Whether no field is known.
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        self.daypart.is_none()
            && self.weekday_type.is_none()
            && self.condition.is_none()
            && self.temperature.is_none()
            && self.device_class.is_none()
            && self.activity.is_none()
            && self.place.is_none()
    }

    /// The packed key (module docs): 0 when nothing is known, otherwise the fields with layout
    /// version [`LAYOUT_VERSION`](Self::LAYOUT_VERSION) in bits 28-31.
    #[must_use]
    pub const fn raw(self) -> u32 {
        if self.is_unknown() {
            return 0;
        }
        let mut raw = (Self::LAYOUT_VERSION as u32) << VERSION_SHIFT;
        if let Some(v) = self.daypart {
            raw |= (v.code() as u32) << DAYPART_SHIFT;
        }
        if let Some(v) = self.weekday_type {
            raw |= (v.code() as u32) << WEEKDAY_SHIFT;
        }
        if let Some(v) = self.condition {
            raw |= (v.code() as u32) << CONDITION_SHIFT;
        }
        if let Some(v) = self.temperature {
            raw |= (v.code() as u32) << TEMPERATURE_SHIFT;
        }
        if let Some(v) = self.device_class {
            raw |= device_code(v) << DEVICE_SHIFT;
        }
        if let Some(v) = self.activity {
            raw |= (v.code() as u32) << ACTIVITY_SHIFT;
        }
        if let Some(v) = self.place {
            raw |= (v.get() as u32) << PLACE_SHIFT;
        }
        raw
    }

    /// Unpacks a stored key.
    ///
    /// # Errors
    /// [`SignalsError::InvalidValue`] (`what` = `"ContextKey"`, `value` = the raw key) for a layout
    /// version other than 1 (a non-zero value with version 0 included), a non-zero reserved
    /// nibble, or a field code with no meaning in version 1.
    pub fn from_raw(raw: u32) -> Result<Self, SignalsError> {
        if raw == 0 {
            return Ok(Self::UNKNOWN);
        }
        let invalid = || SignalsError::InvalidValue {
            what: "ContextKey",
            value: raw,
        };
        if (raw >> VERSION_SHIFT) & VERSION_MASK != u32::from(Self::LAYOUT_VERSION)
            || (raw >> RESERVED_SHIFT) & RESERVED_MASK != 0
        {
            return Err(invalid());
        }
        // A field is masked to at most 6 bits, so the narrowing cast cannot truncate.
        let field = |shift: u32, mask: u32| ((raw >> shift) & mask) as u8;
        // 0 is "unknown"; any other code must name a variant.
        fn decode<T>(
            code: u8,
            from: fn(u8) -> Option<T>,
            invalid: &impl Fn() -> SignalsError,
        ) -> Result<Option<T>, SignalsError> {
            match code {
                0 => Ok(None),
                code => from(code).map(Some).ok_or_else(invalid),
            }
        }
        fn device(code: u8) -> Option<OutputClass> {
            device_from_code(u32::from(code))
        }
        fn place(code: u8) -> Option<PlaceSlot> {
            PlaceSlot::new(code).ok()
        }
        Ok(Self {
            daypart: decode(
                field(DAYPART_SHIFT, DAYPART_MASK),
                Daypart::from_code,
                &invalid,
            )?,
            weekday_type: decode(
                field(WEEKDAY_SHIFT, WEEKDAY_MASK),
                WeekdayType::from_code,
                &invalid,
            )?,
            condition: decode(
                field(CONDITION_SHIFT, CONDITION_MASK),
                WeatherCondition::from_code,
                &invalid,
            )?,
            temperature: decode(
                field(TEMPERATURE_SHIFT, TEMPERATURE_MASK),
                TemperatureBand::from_code,
                &invalid,
            )?,
            device_class: decode(field(DEVICE_SHIFT, DEVICE_MASK), device, &invalid)?,
            activity: decode(
                field(ACTIVITY_SHIFT, ACTIVITY_MASK),
                ActivityGuess::from_code,
                &invalid,
            )?,
            place: decode(field(PLACE_SHIFT, PLACE_MASK), place, &invalid)?,
        })
    }
}

impl Default for ContextKey {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> ContextKey {
        ContextKey::UNKNOWN
            .with_daypart(Daypart::Evening)
            .with_weekday_type(WeekdayType::Weekend)
            .with_condition(WeatherCondition::Rain)
            .with_temperature(TemperatureBand::Mild)
            .with_device_class(OutputClass::Bluetooth)
            .with_activity(ActivityGuess::Leisure)
            .with_place(PlaceSlot::new(5).unwrap())
    }

    /// The version-1 bit layout, written out field by field: this is what is on disk, so a
    /// change here is a layout change and needs a new version.
    #[test]
    fn the_version_1_layout_is_frozen() {
        let expected: u32 = (1 << 28) // version
            | (5 << 18) // place slot
            | (6 << 14) // activity: Leisure
            | (3 << 11) // device class: Bluetooth (OutputClass code 2, plus one)
            | (3 << 8) // temperature band: Mild
            | (3 << 5) // weather condition: Rain
            | (2 << 3) // weekday type: Weekend
            | 6; // daypart: Evening
        assert_eq!(expected, 0x1015_9B76);
        assert_eq!(full().raw(), expected);
    }

    /// Each field alone sets exactly its own bits (plus the version), at the documented place.
    #[test]
    fn each_field_sits_in_its_own_bits() {
        let v = 1u32 << 28;
        let cases = [
            (ContextKey::UNKNOWN.with_daypart(Daypart::LateNight), v | 7),
            (
                ContextKey::UNKNOWN.with_weekday_type(WeekdayType::Holiday),
                v | (3 << 3),
            ),
            (
                ContextKey::UNKNOWN.with_condition(WeatherCondition::Fog),
                v | (6 << 5),
            ),
            (
                ContextKey::UNKNOWN.with_temperature(TemperatureBand::Hot),
                v | (5 << 8),
            ),
            (
                ContextKey::UNKNOWN.with_device_class(OutputClass::Virtual),
                v | (7 << 11),
            ),
            (
                ContextKey::UNKNOWN.with_activity(ActivityGuess::Driving),
                v | (9 << 14),
            ),
            (
                ContextKey::UNKNOWN.with_place(PlaceSlot::new(63).unwrap()),
                v | (63 << 18),
            ),
        ];
        for (key, raw) in cases {
            assert_eq!(key.raw(), raw, "{key:?}");
            assert_eq!(ContextKey::from_raw(raw), Ok(key));
        }
    }

    #[test]
    fn every_value_of_every_field_round_trips() {
        for &d in Daypart::ALL {
            let k = ContextKey::UNKNOWN.with_daypart(d);
            assert_eq!(ContextKey::from_raw(k.raw()), Ok(k));
            assert_eq!(k.daypart(), Some(d));
        }
        for &w in WeekdayType::ALL {
            let k = ContextKey::UNKNOWN.with_weekday_type(w);
            assert_eq!(ContextKey::from_raw(k.raw()), Ok(k));
            assert_eq!(k.weekday_type(), Some(w));
        }
        for &c in WeatherCondition::ALL {
            let k = ContextKey::UNKNOWN.with_condition(c);
            assert_eq!(ContextKey::from_raw(k.raw()), Ok(k));
            assert_eq!(k.condition(), Some(c));
        }
        for &t in TemperatureBand::ALL {
            let k = ContextKey::UNKNOWN.with_temperature(t);
            assert_eq!(ContextKey::from_raw(k.raw()), Ok(k));
            assert_eq!(k.temperature(), Some(t));
        }
        for &c in OutputClass::ALL {
            let k = ContextKey::UNKNOWN.with_device_class(c);
            assert_eq!(ContextKey::from_raw(k.raw()), Ok(k));
            // `Unknown` is the unknown value of the field, not a known class.
            assert_eq!(k.device_class().is_none(), c == OutputClass::Unknown);
        }
        for &a in ActivityGuess::ALL {
            let k = ContextKey::UNKNOWN.with_activity(a);
            assert_eq!(ContextKey::from_raw(k.raw()), Ok(k));
            assert_eq!(k.activity(), Some(a));
        }
        for slot in 1..=PlaceSlot::MAX {
            let k = ContextKey::UNKNOWN.with_place(PlaceSlot::new(slot).unwrap());
            assert_eq!(ContextKey::from_raw(k.raw()), Ok(k));
            assert_eq!(k.place().map(PlaceSlot::get), Some(slot));
        }
        assert_eq!(ContextKey::from_raw(full().raw()), Ok(full()));
    }

    /// Codes are consecutive from 1 in declaration order and never 0; the field widths hold them.
    #[test]
    fn the_field_codes_are_frozen() {
        fn consecutive(codes: Vec<u8>, max: u8) {
            let expected: Vec<u8> = (1..=max).collect();
            assert_eq!(codes, expected);
        }
        consecutive(Daypart::ALL.iter().map(|v| v.code()).collect(), 7);
        consecutive(WeekdayType::ALL.iter().map(|v| v.code()).collect(), 3);
        consecutive(WeatherCondition::ALL.iter().map(|v| v.code()).collect(), 6);
        consecutive(TemperatureBand::ALL.iter().map(|v| v.code()).collect(), 5);
        consecutive(ActivityGuess::ALL.iter().map(|v| v.code()).collect(), 9);
        for &a in ActivityGuess::ALL {
            assert_eq!(ActivityGuess::from_code(a.code()), Some(a));
        }
        assert_eq!(Daypart::from_code(0), None);
        assert_eq!(Daypart::from_code(8), None);
        // The widest enum must fit its field: 9 activities in 4 bits, 7 dayparts in 3 bits.
        assert!(u32::from(ActivityGuess::ALL.len() as u8) <= ACTIVITY_MASK);
        assert!(u32::from(Daypart::ALL.len() as u8) <= DAYPART_MASK);
        assert!(u32::from(WeekdayType::ALL.len() as u8) <= WEEKDAY_MASK);
    }

    /// "Unknown" values: nothing known is the raw value 0 and 0 reads back as nothing known;
    /// unknown fields next to known ones stay unknown.
    #[test]
    fn unknown_is_zero_and_stays_unknown() {
        assert_eq!(ContextKey::UNKNOWN.raw(), 0);
        assert_eq!(ContextKey::default(), ContextKey::UNKNOWN);
        assert_eq!(ContextKey::from_raw(0), Ok(ContextKey::UNKNOWN));
        assert!(ContextKey::UNKNOWN.is_unknown());
        // `OutputClass::Unknown` is not a known device class.
        let k = ContextKey::UNKNOWN.with_device_class(OutputClass::Unknown);
        assert!(k.is_unknown());
        assert_eq!(k.raw(), 0);
        // Only the daypart known: the others read back unknown.
        let k =
            ContextKey::from_raw(ContextKey::UNKNOWN.with_daypart(Daypart::Night).raw()).unwrap();
        assert_eq!(k.daypart(), Some(Daypart::Night));
        assert_eq!(k.weekday_type(), None);
        assert_eq!(k.condition(), None);
        assert_eq!(k.temperature(), None);
        assert_eq!(k.device_class(), None);
        assert_eq!(k.activity(), None);
        assert_eq!(k.place(), None);
        assert!(!k.is_unknown());
        // A version-1 value with every field 0 is also all-unknown (it packs back to 0).
        assert_eq!(ContextKey::from_raw(1 << 28), Ok(ContextKey::UNKNOWN));
    }

    #[test]
    fn values_the_layout_does_not_define_are_rejected() {
        let bad = |raw: u32| {
            assert_eq!(
                ContextKey::from_raw(raw),
                Err(SignalsError::InvalidValue {
                    what: "ContextKey",
                    value: raw
                }),
                "{raw:#010x}"
            );
        };
        let v = 1u32 << 28;
        bad(3); // version 0 with a field set
        bad((2 << 28) | 3); // a version this build does not know
        bad(0xf000_0003);
        bad(v | (1 << 24) | 3); // reserved nibble
        bad(v | (8 << 24));
        bad(v | (7 << 5)); // weather condition 7
        bad(v | (6 << 8)); // temperature band 6
        bad(v | (7 << 8));
        bad(v | (10 << 14)); // activity 10
        bad(v | (15 << 14));
    }

    #[test]
    fn a_place_slot_is_one_to_sixty_three() {
        assert!(PlaceSlot::new(0).is_err());
        assert!(PlaceSlot::new(64).is_err());
        assert_eq!(PlaceSlot::new(1).unwrap().get(), 1);
        assert_eq!(PlaceSlot::new(63).unwrap().get(), 63);
    }

    #[test]
    fn a_packed_key_carries_the_version_in_the_top_nibble() {
        assert_eq!(full().raw() >> 28, u32::from(ContextKey::LAYOUT_VERSION));
        assert_eq!(full().raw() & (0xf << 24), 0, "reserved bits are 0");
    }
}
