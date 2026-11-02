use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Clock, CoreError, Rng, SystemClock, SystemRng, UtcMillis};

/// Largest millisecond value a UUIDv7 can carry (48 bits).
const MAX_V7_MILLIS: i64 = (1 << 48) - 1;
/// `rand_a` is 12 bits wide in a UUIDv7.
const RAND_A_MAX: u16 = 0x0FFF;
/// `rand_b` is 62 bits wide in a UUIDv7.
const RAND_B_MAX: u64 = (1 << 62) - 1;

/// The 74 random bits of a UUIDv7: 12 bits of `rand_a` and 62 bits of `rand_b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rand74 {
    a: u16,
    b: u64,
}

impl Rand74 {
    /// Takes the usable bits of 10 random bytes; the 6 bits a UUIDv7 reserves for version and
    /// variant are ignored.
    fn from_bytes(r: [u8; 10]) -> Self {
        Self {
            a: u16::from_be_bytes([r[0] & 0x0F, r[1]]),
            b: u64::from_be_bytes([r[2] & 0x3F, r[3], r[4], r[5], r[6], r[7], r[8], r[9]]),
        }
    }

    /// The next value in order, or `None` when all 74 bits are set.
    fn successor(self) -> Option<Self> {
        if self.b < RAND_B_MAX {
            Some(Self {
                a: self.a,
                b: self.b + 1,
            })
        } else if self.a < RAND_A_MAX {
            Some(Self {
                a: self.a + 1,
                b: 0,
            })
        } else {
            None
        }
    }
}

/// Validates that `at` fits a UUIDv7 and returns it as unsigned milliseconds.
fn v7_millis(at: UtcMillis) -> Result<u64, CoreError> {
    let ms = at.as_i64();
    if (0..=MAX_V7_MILLIS).contains(&ms) {
        Ok(ms.unsigned_abs())
    } else {
        Err(CoreError::TimestampOutOfRange(ms))
    }
}

/// Lays out a UUIDv7: 48-bit millisecond timestamp, version 7, 74 random bits, variant 10.
fn pack_v7(ms: u64, rand: Rand74) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
    let a = rand.a.to_be_bytes();
    let b = rand.b.to_be_bytes();
    bytes[6] = 0x70 | (a[0] & 0x0F);
    bytes[7] = a[1];
    bytes[8] = 0x80 | (b[0] & 0x3F);
    bytes[9..].copy_from_slice(&b[1..]);
    Uuid::from_bytes(bytes)
}

/// Generates UUIDv7 ids from an injected [`Clock`] and [`Rng`].
///
/// Ids from one generator are strictly increasing, even within one millisecond or if the clock
/// steps backwards: when the clock has not moved past the last id's time, the previous random
/// bits are incremented (RFC 9562 §6.2 Method 2, monotonic random) and the last timestamp is reused.
///
/// Use [`IdGenerator::system`] in production; tests inject
/// [`ManualClock`](crate::ManualClock) and [`SeededRng`](crate::SeededRng).
pub struct IdGenerator {
    clock: Box<dyn Clock>,
    rng: Box<dyn Rng>,
    last: Option<(u64, Rand74)>,
}

impl fmt::Debug for IdGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdGenerator")
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl IdGenerator {
    /// A generator reading `clock` and drawing randomness from `rng`.
    #[must_use]
    pub fn new(clock: impl Clock + 'static, rng: impl Rng + 'static) -> Self {
        Self {
            clock: Box::new(clock),
            rng: Box::new(rng),
            last: None,
        }
    }

    /// A generator on the system clock and system randomness.
    #[must_use]
    pub fn system() -> Self {
        Self::new(SystemClock, SystemRng)
    }

    fn next_uuid(&mut self) -> Result<Uuid, CoreError> {
        let now = v7_millis(self.clock.now())?;
        let (ms, rand) = match self.last {
            Some((last_ms, last_rand)) if now <= last_ms => match last_rand.successor() {
                Some(next) => (last_ms, next),
                // All 74 bits used within one millisecond: move to the next millisecond.
                None => (Self::next_millisecond(last_ms)?, self.fresh_rand()?),
            },
            _ => (now, self.fresh_rand()?),
        };
        self.last = Some((ms, rand));
        Ok(pack_v7(ms, rand))
    }

    fn next_millisecond(last_ms: u64) -> Result<u64, CoreError> {
        let next = last_ms + 1;
        if next > MAX_V7_MILLIS.unsigned_abs() {
            Err(CoreError::TimestampOutOfRange(
                i64::try_from(next).unwrap_or(i64::MAX),
            ))
        } else {
            Ok(next)
        }
    }

    fn fresh_rand(&mut self) -> Result<Rand74, CoreError> {
        let mut bytes = [0u8; 10];
        self.rng.fill_bytes(&mut bytes)?;
        Ok(Rand74::from_bytes(bytes))
    }
}

/// Defines a UUIDv7-backed newtype id.
///
/// Ids are time-ordered: sorting by id sorts by creation time (to the millisecond).
/// Parsing and deserializing reject UUIDs that are not version 7.
/// There is deliberately no `Default` and no constructor that reads the system clock or RNG
/// outside [`IdGenerator::system`].
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);

        impl $name {
            /// Generates the next id from `generator` (its clock time and random bits).
            ///
            /// # Errors
            /// [`CoreError::TimestampOutOfRange`] if the clock is before 1970 or beyond 48 bits;
            /// [`CoreError::RandomUnavailable`] if the generator's `Rng` fails.
            pub fn generate(generator: &mut IdGenerator) -> Result<Self, CoreError> {
                generator.next_uuid().map(Self)
            }

            /// Builds an id from an explicit time and 10 random bytes. Pure and deterministic:
            /// the same inputs give the same id, byte for byte.
            ///
            /// Six of the 80 random bits are discarded because a UUIDv7 stores its version and
            /// variant there (the high nibble of `random[0]` and the top two bits of `random[2]`).
            /// Ids built this way are not ordered relative to each other within a millisecond;
            /// use [`Self::generate`] when that matters.
            ///
            /// # Errors
            /// [`CoreError::TimestampOutOfRange`] if `at` is before 1970 or beyond 48 bits.
            pub fn from_parts(at: UtcMillis, random: [u8; 10]) -> Result<Self, CoreError> {
                Ok(Self(pack_v7(v7_millis(at)?, Rand74::from_bytes(random))))
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

    use super::*;
    use crate::{ManualClock, SeededRng};

    const T0: i64 = 1_700_000_000_000;

    fn generator_at(ms: i64, seed: u64) -> (ManualClock, IdGenerator) {
        let clock = ManualClock::new(UtcMillis::new(ms));
        let generator = IdGenerator::new(clock.clone(), SeededRng::new(seed));
        (clock, generator)
    }

    fn assert_id_traits<T>()
    where
        T: Copy + Eq + Ord + std::hash::Hash + fmt::Debug + fmt::Display,
    {
    }

    /// `check` resolves to exactly one impl only while `T` has no `Default`; with `Default` the
    /// two blanket impls both apply and the call fails to compile (ambiguity).
    trait NotDefault<A> {
        fn check() {}
    }
    impl<T> NotDefault<()> for T {}
    impl<T: Default> NotDefault<u8> for T {}

    #[test]
    fn id_types_do_not_implement_default() {
        <RecordingId as NotDefault<_>>::check();
        <SourceId as NotDefault<_>>::check();
        <SessionId as NotDefault<_>>::check();
        <DeviceId as NotDefault<_>>::check();
        <SignalEventId as NotDefault<_>>::check();
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
    fn generated_ids_are_uuid_v7_with_rfc_variant() {
        let (_, mut g) = generator_at(T0, 1);
        let id = RecordingId::generate(&mut g).unwrap();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert_eq!(id.as_bytes()[8] & 0xC0, 0x80);
    }

    #[test]
    fn ids_from_a_manual_clock_embed_exactly_its_time() {
        let (clock, mut g) = generator_at(T0, 1);
        let a = SessionId::generate(&mut g).unwrap();
        assert_eq!(a.created_at(), UtcMillis::new(T0));
        clock.advance(1);
        let b = SessionId::generate(&mut g).unwrap();
        assert_eq!(b.created_at(), UtcMillis::new(T0 + 1));
    }

    #[test]
    fn advancing_the_clock_orders_ids() {
        let (clock, mut g) = generator_at(T0, 7);
        let mut ids = Vec::new();
        for _ in 0..5 {
            ids.push(RecordingId::generate(&mut g).unwrap());
            clock.advance(3);
        }
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn ids_at_one_instant_are_unique_and_strictly_increasing() {
        let (_, mut g) = generator_at(T0, 3);
        let ids: Vec<SourceId> = (0..1000)
            .map(|_| SourceId::generate(&mut g).unwrap())
            .collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
        assert!(ids.iter().all(|i| i.created_at() == UtcMillis::new(T0)));
    }

    #[test]
    fn ids_of_different_types_share_one_increasing_sequence() {
        let (_, mut g) = generator_at(T0, 3);
        let a = RecordingId::generate(&mut g).unwrap();
        let b = SourceId::generate(&mut g).unwrap();
        assert!(a.as_uuid() < b.as_uuid());
    }

    #[test]
    fn a_backwards_clock_never_breaks_ordering() {
        let (clock, mut g) = generator_at(T0, 3);
        let first = DeviceId::generate(&mut g).unwrap();
        clock.set(UtcMillis::new(T0 - 10_000));
        let second = DeviceId::generate(&mut g).unwrap();
        assert!(first < second);
        assert_eq!(second.created_at(), UtcMillis::new(T0));
    }

    #[test]
    fn random_bit_exhaustion_rolls_into_the_next_millisecond() {
        let (_, mut g) = generator_at(T0, 3);
        let max = Rand74 {
            a: RAND_A_MAX,
            b: RAND_B_MAX,
        };
        g.last = Some((T0.unsigned_abs(), max));
        let before = pack_v7(T0.unsigned_abs(), max);
        let id = RecordingId::generate(&mut g).unwrap();
        assert!(id.as_uuid() > &before);
        assert_eq!(id.created_at(), UtcMillis::new(T0 + 1));
    }

    #[test]
    fn exhaustion_at_the_last_representable_millisecond_is_an_error() {
        let (_, mut g) = generator_at(MAX_V7_MILLIS, 3);
        g.last = Some((
            MAX_V7_MILLIS.unsigned_abs(),
            Rand74 {
                a: RAND_A_MAX,
                b: RAND_B_MAX,
            },
        ));
        assert_eq!(
            RecordingId::generate(&mut g),
            Err(CoreError::TimestampOutOfRange(MAX_V7_MILLIS + 1))
        );
    }

    #[test]
    fn same_time_and_same_random_bits_give_identical_ids() {
        let bits = [0xAB, 0xCD, 0xEF, 1, 2, 3, 4, 5, 6, 7];
        let at = UtcMillis::new(T0);
        let a = RecordingId::from_parts(at, bits).unwrap();
        let b = RecordingId::from_parts(at, bits).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_bytes(), b.as_bytes());
        // Layout pinned byte for byte: 48-bit time, version 7, rand_a, variant 10, rand_b.
        let ms = T0.to_be_bytes();
        assert_eq!(
            a.as_bytes(),
            &[
                ms[2],
                ms[3],
                ms[4],
                ms[5],
                ms[6],
                ms[7],
                0x7B,
                0xCD,
                0x80 | 0x2F,
                1,
                2,
                3,
                4,
                5,
                6,
                7
            ]
        );
        let other = RecordingId::from_parts(at, [0; 10]).unwrap();
        assert_ne!(a, other);
    }

    #[test]
    fn generators_with_equal_clock_and_seed_produce_identical_ids() {
        let (_, mut g1) = generator_at(T0, 99);
        let (_, mut g2) = generator_at(T0, 99);
        for _ in 0..20 {
            assert_eq!(
                SessionId::generate(&mut g1).unwrap(),
                SessionId::generate(&mut g2).unwrap()
            );
        }
        let (_, mut g3) = generator_at(T0, 100);
        let (_, mut g4) = generator_at(T0, 99);
        assert_ne!(
            SessionId::generate(&mut g3).unwrap(),
            SessionId::generate(&mut g4).unwrap()
        );
    }

    #[test]
    fn generate_rejects_a_clock_outside_the_uuid_range() {
        let (clock, mut g) = generator_at(-1, 1);
        assert_eq!(
            RecordingId::generate(&mut g),
            Err(CoreError::TimestampOutOfRange(-1))
        );
        clock.set(UtcMillis::new(1 << 48));
        assert_eq!(
            RecordingId::generate(&mut g),
            Err(CoreError::TimestampOutOfRange(1 << 48))
        );
    }

    #[test]
    fn a_failing_rng_is_reported_not_hidden() {
        struct Broken;
        impl Rng for Broken {
            fn fill_bytes(&mut self, _: &mut [u8]) -> Result<(), CoreError> {
                Err(CoreError::RandomUnavailable("test".to_owned()))
            }
        }
        let mut g = IdGenerator::new(ManualClock::new(UtcMillis::new(T0)), Broken);
        assert_eq!(
            RecordingId::generate(&mut g),
            Err(CoreError::RandomUnavailable("test".to_owned()))
        );
    }

    #[test]
    fn system_generator_makes_recent_increasing_ids() {
        let mut g = IdGenerator::system();
        let ids: Vec<RecordingId> = (0..100)
            .map(|_| RecordingId::generate(&mut g).unwrap())
            .collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        let now = SystemClock.now().as_i64();
        assert!((now - ids[0].created_at().as_i64()).abs() < 5_000);
    }

    #[test]
    fn from_parts_embeds_and_orders_by_the_given_time() {
        let early = SessionId::from_parts(UtcMillis::new(T0), [0xFF; 10]).unwrap();
        let late = SessionId::from_parts(UtcMillis::new(T0 + 1), [0; 10]).unwrap();
        assert_eq!(early.created_at(), UtcMillis::new(T0));
        assert_eq!(late.created_at(), UtcMillis::new(T0 + 1));
        assert!(early < late);
        assert_eq!(early.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn from_parts_handles_epoch_and_max_and_rejects_out_of_range() {
        let r = [5; 10];
        assert_eq!(
            DeviceId::from_parts(UtcMillis::new(0), r)
                .unwrap()
                .created_at(),
            UtcMillis::new(0)
        );
        let max = UtcMillis::new(MAX_V7_MILLIS);
        assert_eq!(DeviceId::from_parts(max, r).unwrap().created_at(), max);
        assert_eq!(
            DeviceId::from_parts(UtcMillis::new(-1), r),
            Err(CoreError::TimestampOutOfRange(-1))
        );
        assert_eq!(
            DeviceId::from_parts(UtcMillis::new(1 << 48), r),
            Err(CoreError::TimestampOutOfRange(1 << 48))
        );
    }

    fn sample<T>(make: fn(UtcMillis, [u8; 10]) -> Result<T, CoreError>, n: u8) -> T {
        make(UtcMillis::new(T0), [n; 10]).unwrap()
    }

    #[test]
    fn display_and_parse_round_trip() {
        let id = sample(RecordingId::from_parts, 1);
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
        let id = sample(DeviceId::from_parts, 2);
        assert_eq!(DeviceId::from_bytes(*id.as_bytes()).unwrap(), id);
        assert!(DeviceId::from_bytes([0; 16]).is_err());
    }

    #[test]
    fn uuid_ids_serde_round_trip_as_plain_strings() {
        let id = sample(RecordingId::from_parts, 3);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<RecordingId>(&json).unwrap(), id);

        let id = sample(SessionId::from_parts, 4);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), id);

        let id = sample(SourceId::from_parts, 5);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<SourceId>(&json).unwrap(), id);

        let id = sample(DeviceId::from_parts, 6);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<DeviceId>(&json).unwrap(), id);
    }

    #[test]
    fn signal_event_id_orders_by_device_then_seq() {
        let d1 = DeviceId::from_parts(UtcMillis::new(1_000), [0; 10]).unwrap();
        let d2 = DeviceId::from_parts(UtcMillis::new(2_000), [0; 10]).unwrap();
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
        let id = SignalEventId::new(sample(DeviceId::from_parts, 7), u64::MAX);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<SignalEventId>(&json).unwrap(), id);
        assert_eq!(id.to_string(), format!("{}#{}", id.device, u64::MAX));
    }
}
