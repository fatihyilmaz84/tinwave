//! The packed byte form of payloads.
//!
//! Layout rules (version 1):
//! - byte 0 is the `payload_version`; fields follow in declaration order, no padding, no tags;
//! - `u8`, `i8`, enums and `bool` take one byte; `u16`/`u32` are LEB128 varints (at most 3/5
//!   bytes); `i16` is a zigzag varint; `Hash32` is four little-endian bytes and `ElapsedDs`
//!   two; `Section` is one byte (label nibble, index nibble); `InputSource` is one byte, or
//!   two for `Remote`;
//! - decoding is strict: unknown enum codes, non-canonical varints, truncation and trailing
//!   bytes are errors, so `encode(decode(bytes)) == bytes` for every accepted input.

use std::fmt;

use crate::enums::{InputSource, Section, SectionKind};
use crate::quant::{ElapsedDs, Fraction, GestureTime, Hash32, PositionDs, Rate, Stars, Volume};
use crate::SignalsError;

/// Hard upper bound for an encoded payload, version byte included.
pub const MAX_PAYLOAD_LEN: usize = 12;

/// An encoded, non-empty payload. Fixed-size and `Copy`: encoding never allocates.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EncodedPayload {
    buf: [u8; MAX_PAYLOAD_LEN],
    len: u8,
}

impl EncodedPayload {
    /// The encoded bytes (1 to 12 of them).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..usize::from(self.len)]
    }

    /// Number of encoded bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// Always `false`: empty payloads are represented as `None`, never as an empty value.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl AsRef<[u8]> for EncodedPayload {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Debug for EncodedPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EncodedPayload(")?;
        for byte in self.as_bytes() {
            write!(f, "{byte:02x}")?;
        }
        write!(f, ")")
    }
}

/// Appends bytes into a fixed 12-byte buffer.
#[derive(Debug)]
pub(crate) struct Writer {
    buf: [u8; MAX_PAYLOAD_LEN],
    len: usize,
}

impl Writer {
    pub(crate) fn new() -> Self {
        Self {
            buf: [0; MAX_PAYLOAD_LEN],
            len: 0,
        }
    }

    pub(crate) fn byte(&mut self, byte: u8) -> Result<(), SignalsError> {
        let slot = self
            .buf
            .get_mut(self.len)
            .ok_or(SignalsError::PayloadTooLong)?;
        *slot = byte;
        self.len += 1;
        Ok(())
    }

    pub(crate) fn varint(&mut self, mut value: u32) -> Result<(), SignalsError> {
        loop {
            let low = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                return self.byte(low);
            }
            self.byte(low | 0x80)?;
        }
    }

    pub(crate) fn finish(self) -> Result<EncodedPayload, SignalsError> {
        let len = u8::try_from(self.len).map_err(|_| SignalsError::PayloadTooLong)?;
        Ok(EncodedPayload { buf: self.buf, len })
    }
}

/// Reads fields from a payload slice.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn byte(&mut self) -> Result<u8, SignalsError> {
        let byte = *self.bytes.get(self.pos).ok_or(SignalsError::Truncated)?;
        self.pos += 1;
        Ok(byte)
    }

    pub(crate) fn varint(&mut self) -> Result<u32, SignalsError> {
        let mut value: u32 = 0;
        for index in 0..5u32 {
            let byte = self.byte()?;
            let bits = u32::from(byte & 0x7f);
            // The fifth byte only has room for the top 4 bits of a u32.
            if index == 4 && bits > 0x0f {
                return Err(SignalsError::VarintOverflow);
            }
            value |= bits << (7 * index);
            if byte & 0x80 == 0 {
                if index > 0 && bits == 0 {
                    return Err(SignalsError::NonCanonicalVarint);
                }
                return Ok(value);
            }
        }
        Err(SignalsError::VarintOverflow)
    }

    pub(crate) fn finish(self) -> Result<(), SignalsError> {
        match self.bytes.len() - self.pos {
            0 => Ok(()),
            extra => Err(SignalsError::TrailingBytes(extra)),
        }
    }
}

/// A field type with a packed byte form.
pub(crate) trait Pack: Sized {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError>;
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError>;
}

impl Pack for u8 {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        w.byte(*self)
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        r.byte()
    }
}

impl Pack for i8 {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        w.byte(self.to_le_bytes()[0])
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        Ok(Self::from_le_bytes([r.byte()?]))
    }
}

impl Pack for bool {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        w.byte(u8::from(*self))
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        match r.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(SignalsError::InvalidValue {
                what: "bool",
                value: u32::from(other),
            }),
        }
    }
}

/// `u16` is a varint (1 to 3 bytes).
impl Pack for u16 {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        w.varint(u32::from(*self))
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        let value = r.varint()?;
        Self::try_from(value).map_err(|_| SignalsError::InvalidValue { what: "u16", value })
    }
}

/// `u32` is a varint (1 to 5 bytes).
impl Pack for u32 {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        w.varint(*self)
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        r.varint()
    }
}

/// `i16` is a zigzag varint (1 to 3 bytes).
impl Pack for i16 {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        let wide = i32::from(*self);
        let zigzag = ((wide << 1) ^ (wide >> 31)) as u32;
        w.varint(zigzag)
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        let zigzag = r.varint()?;
        let wide = ((zigzag >> 1) as i32) ^ -((zigzag & 1) as i32);
        Self::try_from(wide).map_err(|_| SignalsError::InvalidValue {
            what: "i16",
            value: zigzag,
        })
    }
}

impl Pack for Hash32 {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        self.0.to_le_bytes().iter().try_for_each(|b| w.byte(*b))
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        Ok(Self(u32::from_le_bytes([
            r.byte()?,
            r.byte()?,
            r.byte()?,
            r.byte()?,
        ])))
    }
}

macro_rules! pack_raw_u8 {
    ($($ty:ident),*) => {$(
        impl Pack for $ty {
            fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
                w.byte(self.raw())
            }
            fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
                Ok(Self::from_raw(r.byte()?))
            }
        }
    )*};
}
pack_raw_u8!(Volume, Fraction, Rate, GestureTime);

/// Two fixed little-endian bytes, so the worst case never grows past two.
impl Pack for ElapsedDs {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        self.ds().to_le_bytes().iter().try_for_each(|b| w.byte(*b))
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        Ok(Self::from_ds(u16::from_le_bytes([r.byte()?, r.byte()?])))
    }
}

impl Pack for PositionDs {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        self.ds().pack(w)
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        Self::new(u32::unpack(r)?)
    }
}

impl Pack for Stars {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        w.byte(self.get())
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        Self::new(r.byte()?)
    }
}

/// Tag byte of [`InputSource::Remote`]; the remote slot follows as one more byte.
const INPUT_REMOTE_TAG: u8 = 6;
/// Tag byte of [`InputSource::System`].
const INPUT_SYSTEM_TAG: u8 = 7;

impl Pack for InputSource {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        match *self {
            Self::Mouse => w.byte(0),
            Self::Keyboard => w.byte(1),
            Self::MediaKey => w.byte(2),
            Self::Tray => w.byte(3),
            Self::MiniPlayer => w.byte(4),
            Self::Auto => w.byte(5),
            Self::System => w.byte(INPUT_SYSTEM_TAG),
            Self::Remote(slot) => {
                w.byte(INPUT_REMOTE_TAG)?;
                w.byte(slot)
            }
        }
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        match r.byte()? {
            0 => Ok(Self::Mouse),
            1 => Ok(Self::Keyboard),
            2 => Ok(Self::MediaKey),
            3 => Ok(Self::Tray),
            4 => Ok(Self::MiniPlayer),
            5 => Ok(Self::Auto),
            INPUT_REMOTE_TAG => Ok(Self::Remote(r.byte()?)),
            INPUT_SYSTEM_TAG => Ok(Self::System),
            other => Err(SignalsError::InvalidValue {
                what: "InputSource",
                value: u32::from(other),
            }),
        }
    }
}

/// A section is one byte: kind in the low nibble, section index in the high nibble.
impl Pack for Section {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        // `Section::new` guarantees the index fits the high nibble.
        w.byte(self.kind().code() | (self.index() << 4))
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        let byte = r.byte()?;
        Self::new(SectionKind::from_code(byte & 0x0f)?, byte >> 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode<T: Pack>(value: &T) -> Vec<u8> {
        let mut w = Writer::new();
        value.pack(&mut w).unwrap();
        w.finish().unwrap().as_bytes().to_vec()
    }

    fn decode<T: Pack>(bytes: &[u8]) -> Result<T, SignalsError> {
        let mut r = Reader::new(bytes);
        let value = T::unpack(&mut r)?;
        r.finish()?;
        Ok(value)
    }

    #[test]
    fn varint_sizes_and_known_values() {
        assert_eq!(encode(&0u32), [0x00]);
        assert_eq!(encode(&127u32), [0x7f]);
        assert_eq!(encode(&128u32), [0x80, 0x01]);
        assert_eq!(encode(&300u32), [0xac, 0x02]);
        assert_eq!(encode(&u32::MAX), [0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert_eq!(encode(&u16::MAX).len(), 3);
    }

    #[test]
    fn varint_rejects_non_canonical_overflowing_and_truncated() {
        assert_eq!(
            decode::<u32>(&[0x80, 0x00]),
            Err(SignalsError::NonCanonicalVarint)
        );
        assert_eq!(
            decode::<u32>(&[0xff, 0xff, 0xff, 0xff, 0x1f]),
            Err(SignalsError::VarintOverflow)
        );
        assert_eq!(
            decode::<u32>(&[0x80, 0x80, 0x80, 0x80, 0x80]),
            Err(SignalsError::VarintOverflow)
        );
        assert_eq!(decode::<u32>(&[0x80]), Err(SignalsError::Truncated));
        assert!(matches!(
            decode::<u16>(&[0x80, 0x80, 0x04]),
            Err(SignalsError::InvalidValue { what: "u16", .. })
        ));
    }

    #[test]
    fn zigzag_i16_round_trips_extremes() {
        for value in [0i16, 1, -1, 2, -2, 1000, -1000, i16::MAX, i16::MIN] {
            assert_eq!(decode::<i16>(&encode(&value)).unwrap(), value);
        }
        assert_eq!(encode(&-1i16), [0x01]);
        assert_eq!(encode(&1i16), [0x02]);
        assert!(encode(&i16::MIN).len() <= 3);
    }

    #[test]
    fn bool_and_trailing_bytes_are_strict() {
        assert_eq!(decode::<bool>(&[1]), Ok(true));
        assert!(matches!(
            decode::<bool>(&[2]),
            Err(SignalsError::InvalidValue { what: "bool", .. })
        ));
        assert_eq!(decode::<u8>(&[1, 2]), Err(SignalsError::TrailingBytes(1)));
    }

    #[test]
    fn input_source_encodings() {
        assert_eq!(encode(&InputSource::Mouse), [0]);
        assert_eq!(encode(&InputSource::Auto), [5]);
        assert_eq!(encode(&InputSource::Remote(9)), [6, 9]);
        assert_eq!(
            decode::<InputSource>(&[6, 255]),
            Ok(InputSource::Remote(255))
        );
        assert_eq!(encode(&InputSource::System), [7]);
        assert_eq!(decode::<InputSource>(&[7]), Ok(InputSource::System));
        assert!(decode::<InputSource>(&[8]).is_err());
        assert_eq!(decode::<InputSource>(&[6]), Err(SignalsError::Truncated));
    }

    #[test]
    fn section_packs_kind_and_index_into_one_byte() {
        let s = Section::new(SectionKind::Chorus, 3).unwrap();
        assert_eq!(encode(&s), [0x33]);
        assert_eq!(decode::<Section>(&[0x33]), Ok(s));
        assert!(decode::<Section>(&[0x09]).is_err(), "kind 9 does not exist");
    }

    #[test]
    fn writer_refuses_a_thirteenth_byte() {
        let mut w = Writer::new();
        for _ in 0..MAX_PAYLOAD_LEN {
            w.byte(0).unwrap();
        }
        assert_eq!(w.byte(0), Err(SignalsError::PayloadTooLong));
    }
}
