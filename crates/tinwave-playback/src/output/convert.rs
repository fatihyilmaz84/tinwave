//! f32 to device sample formats. Runs inside the audio callback: no allocation, only slices the
//! caller sized up front.
//!
//! Rounding is to nearest with saturation. Scaling is by the full-scale power of two, so a source
//! with `n` bits that Symphonia decoded to f32 (`value / 2^(n-1)`) converts back **bit-exactly** to
//! the same integers: 16-bit material through [`SampleFormat::I16`], 24-bit through
//! [`SampleFormat::I24In32`].
//!
//! # Dither policy (T-0019)
//! [`convert_into`] only rounds. [`convert_dithered`] is what an integer output uses (the
//! [`Pump`](super::Pump) does), and it adds **TPDF dither** (two uniform values, 2 LSB peak to
//! peak) before rounding, so the quantisation error is noise independent of the signal instead of
//! distortion that follows it. The policy, per format:
//!
//! - `F32` (shared mode): never dithered, the samples are passed through bit for bit.
//! - `I32`: never dithered. An `f32` has 24 significant bits, so a conversion to 32 bits loses
//!   nothing there is to dither.
//! - `I16` and `I24In32` (exclusive mode): dithered at 1 LSB of the target format, **except for a
//!   sample that is already exactly an integer of the target format.** Quantising it discards
//!   nothing, so it passes untouched: bit-perfect playback of 16-bit material through `I16` (or
//!   24-bit through `I24In32`) at unity gain stays bit-perfect, and digital silence stays silent.
//!   Anything else (a volume other than 1, resampled audio, 24-bit material into `I16`, a fade)
//!   is off the grid and gets dither.
//!
//! The noise comes from [`Dither`], a small seeded generator: no allocation, no OS randomness, and
//! the same seed gives the same bytes, which tests rely on.

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

impl SampleFormat {
    /// Whether [`convert_dithered`] adds dither for this format (see the module documentation).
    #[must_use]
    pub const fn is_dithered(self) -> bool {
        matches!(self, Self::I16 | Self::I24In32)
    }
}

/// A source of TPDF dither: xorshift64*, seeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dither {
    state: u64,
}

impl Dither {
    /// The seed [`Pump::new`](super::Pump::new) uses.
    pub const DEFAULT_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

    /// A generator starting from `seed` (zero, which xorshift cannot use, is replaced by
    /// [`DEFAULT_SEED`](Self::DEFAULT_SEED)).
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { Self::DEFAULT_SEED } else { seed },
        }
    }

    /// The next noise value in LSB: the difference of two uniform values in [0, 1), so triangular
    /// on (-1, 1) with zero mean.
    fn next_lsb(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        let bits = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // Two disjoint 24-bit fields of the high bits; each is exact in an f32.
        let a = (bits >> 40) as u32;
        let b = ((bits >> 16) & 0x00FF_FFFF) as u32;
        (a as f32 - b as f32) * (1.0 / 16_777_216.0)
    }

    /// `x` (already scaled to LSB) rounded to an integer after dither; an exact integer is left
    /// alone. NaN stays NaN (the caller's saturating cast turns it into 0).
    fn quantise(&mut self, x: f32) -> f32 {
        if x == x.round() {
            x
        } else {
            (x + self.next_lsb()).round()
        }
    }
}

/// Convert `src` to `format` as [`convert_into`] does, adding TPDF dither for the formats that
/// need it (see the module documentation). `F32` and `I32` are converted exactly as
/// [`convert_into`] does and leave `dither` untouched.
pub fn convert_dithered(format: SampleFormat, src: &[f32], dst: &mut [u8], dither: &mut Dither) {
    match format {
        SampleFormat::I16 => {
            for (s, out) in src.iter().zip(dst.chunks_exact_mut(2)) {
                // Float to int `as` casts saturate and map NaN to 0.
                let v = dither.quantise(s * 32_768.0) as i16;
                out.copy_from_slice(&v.to_le_bytes());
            }
        }
        SampleFormat::I24In32 => {
            for (s, out) in src.iter().zip(dst.chunks_exact_mut(4)) {
                let v = dither
                    .quantise(s * 8_388_608.0)
                    .clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.copy_from_slice(&(v << 8).to_le_bytes());
            }
        }
        SampleFormat::F32 | SampleFormat::I32 => convert_into(format, src, dst),
    }
}

/// Convert `src` to `format`, little endian, into `dst`, rounding to nearest and never dithering.
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

    // ---- dither ---------------------------------------------------------------------------

    fn run_dithered(format: SampleFormat, src: &[f32], dither: &mut Dither) -> Vec<u8> {
        let mut dst = vec![0u8; src.len() * format.bytes_per_sample()];
        convert_dithered(format, src, &mut dst, dither);
        dst
    }

    fn i16s(bytes: &[u8]) -> Vec<i32> {
        bytes
            .chunks_exact(2)
            .map(|c| i32::from(i16::from_le_bytes([c[0], c[1]])))
            .collect()
    }

    /// Off-grid test signal for every format: values between the 16-bit steps.
    fn off_grid() -> Vec<f32> {
        (0..4_000)
            .map(|n| (n as f32 * 0.37 + 0.3).sin() * 0.7 + 1.0e-5 * n as f32)
            .collect()
    }

    #[test]
    fn f32_and_i32_are_never_dithered() {
        let src = off_grid();
        for format in [SampleFormat::F32, SampleFormat::I32] {
            assert!(!format.is_dithered());
            let mut dither = Dither::new(7);
            let before = dither.clone();
            let got = run_dithered(format, &src, &mut dither);
            assert_eq!(
                got,
                run(format, &src),
                "{format:?} must equal plain conversion"
            );
            assert_eq!(dither, before, "{format:?} must not even draw noise");
        }
        assert!(SampleFormat::I16.is_dithered() && SampleFormat::I24In32.is_dithered());
    }

    #[test]
    fn samples_already_on_the_grid_are_left_alone() {
        // Every 16-bit value through I16: bit-perfect even though dither is on.
        let src: Vec<f32> = (-32_768i32..=32_767).map(|n| n as f32 / 32_768.0).collect();
        let mut dither = Dither::new(1);
        let before = dither.clone();
        let bytes = run_dithered(SampleFormat::I16, &src, &mut dither);
        assert_eq!(bytes, run(SampleFormat::I16, &src));
        assert_eq!(dither, before, "an exact sample draws no noise");

        // 24-bit values through I24In32, and digital silence in either.
        let values = [-8_388_608i32, -1, 0, 1, 12_345, 8_388_607];
        let src: Vec<f32> = values.iter().map(|&n| n as f32 / 8_388_608.0).collect();
        let bytes = run_dithered(SampleFormat::I24In32, &src, &mut dither);
        assert_eq!(bytes, run(SampleFormat::I24In32, &src));
        let silence = run_dithered(SampleFormat::I16, &[0.0; 64], &mut dither);
        assert!(silence.iter().all(|&b| b == 0));
        assert_eq!(dither, before);
    }

    #[test]
    fn off_grid_samples_get_triangular_dither_that_keeps_the_mean() {
        // A constant 0.3 LSB: plain rounding says 0 forever; TPDF dither says -1, 0 or 1 with
        // probabilities 0.02 / 0.66 / 0.32, so the average is 0.3.
        let n = 400_000;
        let src = vec![0.3 / 32_768.0; n];
        assert!(run(SampleFormat::I16, &src).iter().all(|&b| b == 0));

        let out = i16s(&run_dithered(SampleFormat::I16, &src, &mut Dither::new(42)));
        let count = |v: i32| out.iter().filter(|&&x| x == v).count() as f64 / n as f64;
        assert!(out.iter().all(|&x| (-1..=1).contains(&x)));
        assert!((count(-1) - 0.02).abs() < 0.005, "P(-1) = {}", count(-1));
        assert!((count(0) - 0.66).abs() < 0.005, "P(0) = {}", count(0));
        assert!((count(1) - 0.32).abs() < 0.005, "P(1) = {}", count(1));
        let mean = out.iter().map(|&x| f64::from(x)).sum::<f64>() / n as f64;
        assert!((mean - 0.3).abs() < 0.005, "mean {mean}");

        // The same at 24 bits (1 LSB of that format).
        let src = vec![0.3 / 8_388_608.0; n];
        let bytes = run_dithered(SampleFormat::I24In32, &src, &mut Dither::new(42));
        let mean = bytes
            .chunks_exact(4)
            .map(|c| f64::from(i32::from_le_bytes([c[0], c[1], c[2], c[3]]) >> 8))
            .sum::<f64>()
            / n as f64;
        assert!((mean - 0.3).abs() < 0.005, "24-bit mean {mean}");
    }

    #[test]
    fn dither_differs_from_plain_rounding_by_at_most_one_step() {
        let src = off_grid();
        let plain = i16s(&run(SampleFormat::I16, &src));
        let dithered = i16s(&run_dithered(SampleFormat::I16, &src, &mut Dither::new(3)));
        let differing = plain.iter().zip(&dithered).filter(|(a, b)| a != b).count();
        assert!(
            differing > src.len() / 10,
            "only {differing} samples differ"
        );
        assert!(plain.iter().zip(&dithered).all(|(a, b)| (a - b).abs() <= 1));
    }

    #[test]
    fn the_same_seed_gives_the_same_bytes() {
        let src = off_grid();
        let a = run_dithered(SampleFormat::I16, &src, &mut Dither::new(99));
        assert_eq!(
            a,
            run_dithered(SampleFormat::I16, &src, &mut Dither::new(99))
        );
        assert_ne!(
            a,
            run_dithered(SampleFormat::I16, &src, &mut Dither::new(100))
        );
        assert_eq!(Dither::new(0), Dither::new(Dither::DEFAULT_SEED));
    }

    #[test]
    fn dithered_conversion_still_saturates_and_maps_nan_to_silence() {
        let src = [2.0, -2.0, f32::NAN, f32::INFINITY, -f32::INFINITY];
        let mut dither = Dither::new(5);
        let v = i16s(&run_dithered(SampleFormat::I16, &src, &mut dither));
        assert_eq!(v, [32_767, -32_768, 0, 32_767, -32_768]);
        let bytes = run_dithered(SampleFormat::I24In32, &src, &mut dither);
        let v: Vec<i32> = bytes.chunks_exact(4).map(read32).collect();
        assert_eq!(
            v,
            [
                8_388_607 << 8,
                -8_388_608 << 8,
                0,
                8_388_607 << 8,
                -8_388_608 << 8
            ]
        );
    }
}
