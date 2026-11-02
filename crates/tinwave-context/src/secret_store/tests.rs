//! Secret store tests. The default run uses only the in-memory fake; the one test that talks to the
//! real Windows Credential Manager is `#[ignore]`d and uses a random test-only target name.

use std::cell::{Cell, RefCell};

use tinwave_core::SystemRng;

use super::*;

/// An in-memory store that counts its writes.
#[derive(Default)]
struct FakeStore {
    entry: RefCell<Option<Vec<u8>>>,
    writes: Cell<u32>,
}

impl FakeStore {
    fn holding(bytes: &[u8]) -> Self {
        Self {
            entry: RefCell::new(Some(bytes.to_vec())),
            writes: Cell::new(0),
        }
    }
}

impl SecretStore for FakeStore {
    fn read(&self) -> Result<Option<Vec<u8>>, SecretStoreError> {
        Ok(self.entry.borrow().clone())
    }

    fn write(&self, bytes: &[u8]) -> Result<(), SecretStoreError> {
        self.writes.set(self.writes.get() + 1);
        *self.entry.borrow_mut() = Some(bytes.to_vec());
        Ok(())
    }
}

/// A store that never keeps what it is given.
struct ForgetfulStore;

impl SecretStore for ForgetfulStore {
    fn read(&self) -> Result<Option<Vec<u8>>, SecretStoreError> {
        Ok(None)
    }

    fn write(&self, _bytes: &[u8]) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

/// A store that is unreachable.
struct BrokenStore;

impl SecretStore for BrokenStore {
    fn read(&self) -> Result<Option<Vec<u8>>, SecretStoreError> {
        Err(SecretStoreError::Os("keychain locked".to_owned()))
    }

    fn write(&self, _bytes: &[u8]) -> Result<(), SecretStoreError> {
        Err(SecretStoreError::Os("keychain locked".to_owned()))
    }
}

#[test]
fn the_first_run_creates_and_stores_a_secret() {
    let store = FakeStore::default();
    let secret = store.load_or_create(&mut SystemRng).unwrap();
    assert_eq!(store.writes.get(), 1);
    assert_eq!(
        store.entry.borrow().as_deref(),
        Some(secret.as_bytes().as_slice())
    );
}

#[test]
fn later_runs_load_the_same_bytes_and_write_nothing() {
    let store = FakeStore::default();
    let first = store.load_or_create(&mut SystemRng).unwrap();
    let second = store.load_or_create(&mut SystemRng).unwrap();
    let third = store.load_or_create(&mut SystemRng).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, third);
    assert_eq!(store.writes.get(), 1);
}

#[test]
fn an_existing_secret_is_loaded_not_replaced() {
    let store = FakeStore::holding(&[9; DEVICE_SECRET_LEN]);
    let secret = store.load_or_create(&mut SystemRng).unwrap();
    assert_eq!(secret, DeviceSecret::from_bytes([9; DEVICE_SECRET_LEN]));
    assert_eq!(store.writes.get(), 0);
}

#[test]
fn a_wrong_size_entry_is_an_error_and_is_never_regenerated() {
    for bad in [
        &[][..],
        &[1u8; 1][..],
        &[1u8; DEVICE_SECRET_LEN - 1][..],
        &[1u8; DEVICE_SECRET_LEN + 1][..],
        &[1u8; 4096][..],
    ] {
        let store = FakeStore::holding(bad);
        let error = store.load_or_create(&mut SystemRng).unwrap_err();
        assert!(
            matches!(
                error,
                SecretStoreError::Corrupt { expected, found }
                    if expected == DEVICE_SECRET_LEN && found == bad.len()
            ),
            "{error}"
        );
        assert_eq!(store.writes.get(), 0, "must not overwrite a corrupt entry");
        assert_eq!(store.entry.borrow().as_deref(), Some(bad));
    }
}

#[test]
fn the_corrupt_message_names_the_sizes_and_never_the_bytes() {
    let store = FakeStore::holding(&[0xAB; 7]);
    let error = store.load_or_create(&mut SystemRng).unwrap_err();
    let text = format!("{error} / {error:?}");
    assert!(text.contains("expected 32 bytes, found 7"), "{text}");
    assert!(!text.contains("171") && !text.to_lowercase().contains("abab"));
}

#[test]
fn an_unreachable_store_is_an_error_not_a_throwaway_secret() {
    let error = BrokenStore.load_or_create(&mut SystemRng).unwrap_err();
    assert!(matches!(error, SecretStoreError::Os(_)), "{error}");
}

#[test]
fn a_store_that_forgets_the_new_secret_is_noticed() {
    let error = ForgetfulStore.load_or_create(&mut SystemRng).unwrap_err();
    assert!(matches!(error, SecretStoreError::Os(_)), "{error}");
}

#[test]
fn the_loaded_secret_never_shows_its_bytes() {
    let secret = FakeStore::default().load_or_create(&mut SystemRng).unwrap();
    assert_eq!(format!("{secret:?}"), "DeviceSecret(<redacted>)");
}

/// Off Windows every operation reports `Unsupported`; it never fakes a secret.
#[cfg(not(windows))]
#[test]
fn other_oses_report_unsupported() {
    let store = OsSecretStore::device_secret();
    assert!(matches!(
        store.read(),
        Err(SecretStoreError::Unsupported(reason)) if !reason.is_empty()
    ));
    assert!(matches!(
        store.write(&[0; DEVICE_SECRET_LEN]),
        Err(SecretStoreError::Unsupported(_))
    ));
    assert!(matches!(
        store.load_or_create(&mut SystemRng),
        Err(SecretStoreError::Unsupported(_))
    ));
}

#[cfg(windows)]
mod live {
    use tinwave_core::Rng;

    use super::*;

    /// Deletes its test-only credential when dropped, even if the test panics.
    struct TestCredential(String);

    impl TestCredential {
        /// A target no real install uses: `Tinwave/test-device-secret-<random hex>`.
        fn new() -> Self {
            let mut random = [0u8; 8];
            SystemRng.fill_bytes(&mut random).unwrap();
            let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let target = format!("Tinwave/test-device-secret-{hex}");
            assert_ne!(target, DEVICE_SECRET_TARGET);
            Self(target)
        }

        fn store(&self) -> OsSecretStore {
            OsSecretStore::with_target(self.0.clone())
        }
    }

    impl Drop for TestCredential {
        fn drop(&mut self) {
            // A missing credential is fine (the test may have failed before writing it).
            if let Err(error) = win::delete(&self.0) {
                eprintln!("cleanup of the test credential {}: {error}", self.0);
            }
        }
    }

    /// Writes, reads and deletes a test-only credential in the real Windows Credential Manager.
    /// Run with `cargo test -p tinwave-context -- --ignored secret_store::tests::live`.
    #[test]
    #[ignore = "uses the real Windows Credential Manager (test-only target name)"]
    fn credential_manager_round_trip() {
        let credential = TestCredential::new();
        let store = credential.store();

        assert!(store.read().unwrap().is_none(), "fresh target is empty");

        let first = store.load_or_create(&mut SystemRng).unwrap();
        let second = store.load_or_create(&mut SystemRng).unwrap();
        assert_eq!(first, second, "the second run loads the first run's bytes");
        assert_eq!(
            store.read().unwrap().as_deref(),
            Some(first.as_bytes().as_slice())
        );

        // A corrupt entry is a visible error and stays exactly as it was.
        store.write(&[5; 5]).unwrap();
        let error = store.load_or_create(&mut SystemRng).unwrap_err();
        assert!(
            matches!(error, SecretStoreError::Corrupt { found: 5, .. }),
            "{error}"
        );
        assert_eq!(store.read().unwrap().as_deref(), Some(&[5u8; 5][..]));

        win::delete(&credential.0).unwrap();
        assert!(store.read().unwrap().is_none(), "deleted target is gone");
    }
}
