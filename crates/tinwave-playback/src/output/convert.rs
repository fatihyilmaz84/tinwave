//! f32 to device sample formats. Runs inside the audio callback: no allocation, only slices the
//! caller sized up front.
//!
//! Rounding is to nearest with saturation. There is no dither or noise shaping (T-0019 decides
//! that policy). Scaling is by the full-scale power of two, so a source with `n` bits that
//! Symphonia decoded to f32 (`value / 2^(n-1)`) converts back **bit-exactly** to the same
//! integers: 16-bit material through [`SampleFormat::I16`], 24-bit through
//! [`SampleFormat::I24In32`].

/// A sample format a device can be fed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// 32-bit IEEE float (shared mode).
    F32,
    /// 16-bit signed integer.
    I16,
    /// 24 valid bits MSB-aligned in a 32-bit container (the low byte is zero).
    I24In32,
    /// 32-bit signed integer.
    I32,
}

impl SampleFormat {
    /// Bytes per sample (one channel).
    #[must_use]
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::F32 | Self::I24In32 | Self::I32 => 4,
        }
    }

    /// Valid bits per sample.
    #[must_use]
    pub const fn valid_bits(self) -> usize {
        match self {
            Self::I16 => 16,
            Self::I24In32 => 24,
            Self::F32 | Self::I32 => 32,
        }
    }
}

/// Convert `src` to `format`, little endian, into `dst`.
///
/// `dst.len()` should be `src.len() * format.bytes_per_sample()`; a short `dst` converts only the
/// samples that fit and extra bytes are left untouched.
pub fn convert_into(format: SampleFormat, src: &[f32], dst: &mut [u8]) {
    match format {
        SampleFormat::F32 => {
            for (s, out) in src.iter().zip(dst.chunks_exact_mut(4)) {
                out.copy_from_slice(&s.to_le_bytes());
            }
        }
        SampleFormat::I16 => {
            for (s, out) in src.iter().zip(dst.chunks_exact_mut(2)) {
                // Float to int `as` casts saturate and map NaN to 0.
                let v = (s * 32_768.0).round() as i16;
                out.copy_from_slice(&v.to_le_bytes());
            }
        }
        SampleFormat::I24In32 => {
            for (s, out) in src.iter().zip(dst.chunks_exact_mut(4)) {
                let v = (s * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.copy_from_slice(&(v << 8).to_le_bytes());
            }
        }
        SampleFormat::I32 => {
            for (s, out) in src.iter().zip(dst.chunks_exact_mut(4)) {
                let v = (f64::from(*s) * 2_147_483_648.0).round() as i32;
                out.copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(format: SampleFormat, src: &[f32]) -> Vec<u8> {
        let mut dst = vec![0u8; src.len() * format.bytes_per_sample()];
        convert_into(format, src, &mut dst);
        dst
    }

    fn read32(chunk: &[u8]) -> i32 {
        i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
    }

    #[test]
    fn i16_roundtrips_every_16_bit_value() {
        let src: Vec<f32> = (-32_768i32..=32_767).map(|n| n as f32 / 32_768.0).collect();
        let bytes = run(SampleFormat::I16, &src);
        for (n, chunk) in (-32_768i32..=32_767).zip(bytes.chunks_exact(2)) {
            assert_eq!(i16::from_le_bytes([chunk[0], chunk[1]]), n as i16);
        }
    }

    #[test]
    fn i24_roundtrips_24_bit_values_and_keeps_the_low_byte_zero() {
        let values = [
            -8_388_608i32,
            -8_388_607,
            -1,
            0,
            1,
            12_345,
            8_388_606,
            8_388_607,
        ];
        let src: Vec<f32> = values.iter().map(|&n| n as f32 / 8_388_608.0).collect();
        let bytes = run(SampleFormat::I24In32, &src);
        for (n, chunk) in values.iter().zip(bytes.chunks_exact(4)) {
            let v = read32(chunk);
            assert_eq!(v >> 8, *n);
            assert_eq!(v & 0xFF, 0);
        }
    }

    #[test]
    fn i32_carries_24_bit_values_in_the_top_bits() {
        let values = [-8_388_608i32, -5, 0, 7, 8_388_607];
        let src: Vec<f32> = values.iter().map(|&n| n as f32 / 8_388_608.0).collect();
        let bytes = run(SampleFormat::I32, &src);
        for (n, chunk) in values.iter().zip(bytes.chunks_exact(4)) {
            assert_eq!(read32(chunk), n << 8);
        }
    }

    #[test]
    fn f32_is_passed_through_bit_exactly() {
        let src = [0.0f32, -0.0, 0.1, -0.999_999, 1.0, f32::MIN_POSITIVE];
        let bytes = run(SampleFormat::F32, &src);
        for (s, chunk) in src.iter().zip(bytes.chunks_exact(4)) {
            assert_eq!(chunk, &s.to_le_bytes());
        }
    }

    #[test]
    fn out_of_range_saturates_and_nan_is_silence() {
        let bytes = run(SampleFormat::I16, &[2.0, -2.0, f32::NAN, f32::INFINITY]);
        let v = |i: usize| i16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
        assert_eq!((v(0), v(1), v(2), v(3)), (i16::MAX, i16::MIN, 0, i16::MAX));

        let bytes = run(SampleFormat::I24In32, &[2.0, -2.0, f32::NAN, f32::INFINITY]);
        let v = |i: usize| read32(&bytes[i * 4..i * 4 + 4]);
        assert_eq!(v(0), 8_388_607 << 8);
        assert_eq!(v(1), -8_388_608 << 8);
        assert_eq!((v(2), v(3)), (0, 8_388_607 << 8));

        let bytes = run(SampleFormat::I32, &[2.0, -2.0, f32::NAN, f32::INFINITY]);
        let v = |i: usize| read32(&bytes[i * 4..i * 4 + 4]);
        assert_eq!((v(0), v(1), v(2), v(3)), (i32::MAX, i32::MIN, 0, i32::MAX));
    }

    #[test]
    fn a_short_destination_converts_only_what_fits() {
        let mut dst = [0xAAu8; 3];
        convert_into(SampleFormat::I16, &[0.5, 0.5], &mut dst);
        assert_eq!(dst[2], 0xAA, "the odd trailing byte is untouched");
    }
}
