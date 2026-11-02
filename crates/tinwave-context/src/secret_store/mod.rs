//! Where the [`DeviceSecret`] lives: the OS keychain (`docs/rules/privacy-and-signals.md`).
//!
//! The secret salts every device hash, so losing or regenerating it silently re-keys all of
//! them. That decides the rules here:
//!
//! - The first run generates the secret with [`SystemRng`](tinwave_core::SystemRng) (the only
//!   `CryptoRng`) and stores it; every later run loads the same bytes.
//! - An entry that is not exactly [`DEVICE_SECRET_LEN`] bytes is a [`SecretStoreError::Corrupt`]
//!   the caller must show. It is never replaced by a fresh secret.
//! - A store that cannot be reached is an error too, never a reason to run with a throwaway
//!   secret.
//! - Secret bytes are never logged and never part of an error or `Debug` output.
//!
//! Windows uses Credential Manager (`win`, the only `unsafe` here). macOS and Linux report
//! [`SecretStoreError::Unsupported`] until their own tickets land.

use thiserror::Error;
use tinwave_core::{CoreError, CryptoRng};

use crate::audio::{DeviceSecret, DEVICE_SECRET_LEN};

#[cfg(windows)]
mod win;

/// The credential target of the real device secret. Tests use their own target names.
#[cfg(windows)]
const DEVICE_SECRET_TARGET: &str = "Tinwave/device-secret";

/// Why the device secret could not be loaded or created.
#[derive(Debug, Error)]
pub enum SecretStoreError {
    /// This OS has no secret store implementation yet.
    #[error("no secret store is implemented for this OS: {0}")]
    Unsupported(&'static str),
    /// A stored entry exists but is not a valid secret. It was left untouched.
    #[error(
        "the stored device secret is corrupt (expected {expected} bytes, found {found}); \
         it was not replaced because that would change every device hash"
    )]
    Corrupt {
        /// The required length.
        expected: usize,
        /// The length found.
        found: usize,
    },
    /// The OS keychain refused or failed the request.
    #[error("the OS keychain failed: {0}")]
    Os(String),
    /// A new secret could not be generated.
    #[error("could not generate a device secret: {0}")]
    Random(#[from] CoreError),
}

/// Raw access to one secret entry. Implementations store bytes; [`load_or_create`] decides what
/// they mean.
///
/// [`load_or_create`]: SecretStore::load_or_create
pub trait SecretStore {
    /// The stored bytes, or `None` if nothing is stored yet.
    ///
    /// # Errors
    /// [`SecretStoreError::Os`] or [`SecretStoreError::Unsupported`]; a missing entry is `Ok(None)`.
    fn read(&self) -> Result<Option<Vec<u8>>, SecretStoreError>;

    /// Stores `bytes`, replacing any previous entry.
    ///
    /// # Errors
    /// [`SecretStoreError::Os`] or [`SecretStoreError::Unsupported`].
    fn write(&self, bytes: &[u8]) -> Result<(), SecretStoreError>;

    /// Loads the device secret, creating and storing one on the very first run.
    ///
    /// After a first-run write the entry is read back and the stored value wins, so two
    /// processes starting together end up with the same secret and a store that silently drops
    /// writes is noticed.
    ///
    /// # Errors
    /// - [`SecretStoreError::Corrupt`] if an entry exists with the wrong size (nothing is
    ///   regenerated or overwritten).
    /// - [`SecretStoreError::Os`] / [`SecretStoreError::Unsupported`] from the store.
    /// - [`SecretStoreError::Random`] if the system cannot produce randomness.
    fn load_or_create(&self, rng: &mut impl CryptoRng) -> Result<DeviceSecret, SecretStoreError> {
        if let Some(bytes) = self.read()? {
            return secret_from(&bytes);
        }
        let fresh = DeviceSecret::generate(rng)?;
        self.write(fresh.as_bytes())?;
        match self.read()? {
            Some(bytes) => secret_from(&bytes),
            None => Err(SecretStoreError::Os(
                "the new device secret was not found after writing it".to_owned(),
            )),
        }
    }
}

fn secret_from(bytes: &[u8]) -> Result<DeviceSecret, SecretStoreError> {
    let array: [u8; DEVICE_SECRET_LEN] =
        bytes.try_into().map_err(|_| SecretStoreError::Corrupt {
            expected: DEVICE_SECRET_LEN,
            found: bytes.len(),
        })?;
    Ok(DeviceSecret::from_bytes(array))
}

/// The secret store of this OS: Windows Credential Manager, or [`SecretStoreError::Unsupported`]
/// everywhere else.
#[derive(Debug, Clone)]
pub struct OsSecretStore {
    #[cfg(windows)]
    target: String,
}

impl OsSecretStore {
    /// The store of the real per-user device secret (`Tinwave/device-secret` on Windows).
    #[must_use]
    pub fn device_secret() -> Self {
        Self {
            #[cfg(windows)]
            target: DEVICE_SECRET_TARGET.to_owned(),
        }
    }

    /// A store on another credential target. Only tests use it, so they never touch the real entry.
    #[cfg(all(windows, test))]
    fn with_target(target: String) -> Self {
        Self { target }
    }
}

#[cfg(windows)]
impl SecretStore for OsSecretStore {
    fn read(&self) -> Result<Option<Vec<u8>>, SecretStoreError> {
        win::read(&self.target)
    }

    fn write(&self, bytes: &[u8]) -> Result<(), SecretStoreError> {
        win::write(&self.target, bytes)
    }
}

#[cfg(not(windows))]
impl SecretStore for OsSecretStore {
    fn read(&self) -> Result<Option<Vec<u8>>, SecretStoreError> {
        Err(SecretStoreError::Unsupported(UNSUPPORTED_REASON))
    }

    fn write(&self, _bytes: &[u8]) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Unsupported(UNSUPPORTED_REASON))
    }
}

/// Why there is no secret store on this OS.
#[cfg(not(windows))]
const UNSUPPORTED_REASON: &str = "the device secret keychain is only implemented for Windows";

#[cfg(test)]
mod tests;
