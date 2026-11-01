use crate::CoreError;

/// A source of random bytes.
///
/// Domain code takes an `Rng` instead of reaching for the operating system, so replay and golden
/// tests can pin every random value. Ids need uniqueness, not secrecy.
pub trait Rng: Send {
    /// Fills `dest` with random bytes.
    ///
    /// # Errors
    /// [`CoreError::RandomUnavailable`] if the source cannot produce randomness.
    fn fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), CoreError>;
}

/// Operating-system randomness. The only place in `tinwave-core` that touches the system RNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SystemRng;

impl Rng for SystemRng {
    fn fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), CoreError> {
        getrandom::fill(dest).map_err(|e| CoreError::RandomUnavailable(e.to_string()))
    }
}

/// A deterministic generator: the same seed always yields the same byte stream.
///
/// For tests, replays and benchmarks. It is **not** cryptographic and must not be used for secrets.
/// (SplitMix64.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    /// A generator starting from `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl Rng for SeededRng {
    fn fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), CoreError> {
        for chunk in dest.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_rng_is_reproducible_and_seed_dependent() {
        let mut a = SeededRng::new(42);
        let mut b = SeededRng::new(42);
        let mut c = SeededRng::new(43);
        let (mut x, mut y, mut z) = ([0u8; 13], [0u8; 13], [0u8; 13]);
        a.fill_bytes(&mut x).unwrap();
        b.fill_bytes(&mut y).unwrap();
        c.fill_bytes(&mut z).unwrap();
        assert_eq!(x, y);
        assert_ne!(x, z);
    }

    #[test]
    fn seeded_rng_stream_advances() {
        let mut rng = SeededRng::new(1);
        let (mut a, mut b) = ([0u8; 10], [0u8; 10]);
        rng.fill_bytes(&mut a).unwrap();
        rng.fill_bytes(&mut b).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn system_rng_produces_varying_bytes() {
        let mut rng = SystemRng;
        let (mut a, mut b) = ([0u8; 16], [0u8; 16]);
        rng.fill_bytes(&mut a).unwrap();
        rng.fill_bytes(&mut b).unwrap();
        assert_ne!(a, b);
    }
}
