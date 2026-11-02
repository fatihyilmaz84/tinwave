//! Device identity: `device_hash = blake3(device_secret || endpoint_id)` (ADR 0004 section 5).
//!
//! The endpoint id is an OS string like `{0.0.0.00000000}.{guid}`. It is hashed on the provider
//! thread and never stored or logged; the friendly name and the container id are never read.
//! The secret is generated once per install and kept by the caller (the OS keychain, see
//! `docs/rules/privacy-and-signals.md`); without it the 32-bit hash cannot be matched back to an
//! endpoint id by anyone who only has the signal database.

use std::fmt;

use tinwave_core::{CoreError, CryptoRng};
use tinwave_signals::Hash32;

/// Length of the secret in bytes.
pub const DEVICE_SECRET_LEN: usize = 32;

/// The per-install salt of every device hash.
///
/// Generate it with [`DeviceSecret::generate`] (which only accepts a [`CryptoRng`]) and persist
/// [`DeviceSecret::as_bytes`] in the keychain; restore it with [`DeviceSecret::from_bytes`].
/// `Debug` never prints the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceSecret([u8; DEVICE_SECRET_LEN]);

impl DeviceSecret {
    /// A fresh random secret.
    ///
    /// # Errors
    /// [`CoreError::RandomUnavailable`] if the operating system cannot produce randomness.
    pub fn generate(rng: &mut impl CryptoRng) -> Result<Self, CoreError> {
        let mut bytes = [0u8; DEVICE_SECRET_LEN];
        rng.fill_bytes(&mut bytes)?;
        Ok(Self(bytes))
    }

    /// A secret restored from storage.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; DEVICE_SECRET_LEN]) -> Self {
        Self(bytes)
    }

    /// The bytes to persist. Treat them like a password.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DEVICE_SECRET_LEN] {
        &self.0
    }

    /// The first 32 bits (little endian) of `blake3(secret || endpoint_id)`.
    #[must_use]
    pub fn device_hash(&self, endpoint_id: &str) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&self.0);
        hasher.update(endpoint_id.as_bytes());
        let digest = hasher.finalize();
        let b = digest.as_bytes();
        Hash32(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

impl fmt::Debug for DeviceSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeviceSecret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinwave_core::SystemRng;

    const ID: &str = "{0.0.0.00000000}.{11111111-2222-3333-4444-555555555555}";

    #[test]
    fn the_hash_is_the_documented_blake3_prefix() {
        let secret = DeviceSecret::from_bytes([7; DEVICE_SECRET_LEN]);
        let mut input = vec![7u8; DEVICE_SECRET_LEN];
        input.extend_from_slice(ID.as_bytes());
        let digest = blake3::hash(&input);
        let b = digest.as_bytes();
        assert_eq!(
            secret.device_hash(ID),
            Hash32(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        );
    }

    #[test]
    fn the_hash_depends_on_both_the_secret_and_the_endpoint() {
        let a = DeviceSecret::from_bytes([1; DEVICE_SECRET_LEN]);
        let b = DeviceSecret::from_bytes([2; DEVICE_SECRET_LEN]);
        assert_eq!(a.device_hash(ID), a.device_hash(ID));
        assert_ne!(a.device_hash(ID), b.device_hash(ID));
        assert_ne!(a.device_hash(ID), a.device_hash("{0.0.0.00000000}.{other}"));
    }

    #[test]
    fn generated_secrets_differ_and_debug_hides_them() {
        let mut rng = SystemRng;
        let a = DeviceSecret::generate(&mut rng).unwrap();
        let b = DeviceSecret::generate(&mut rng).unwrap();
        assert_ne!(a, b);
        assert_eq!(format!("{a:?}"), "DeviceSecret(<redacted>)");
    }
}
