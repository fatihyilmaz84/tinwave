//! Windows Credential Manager: the only module of the secret store that uses `unsafe`.
//!
//! One generic credential per target, persisted for the current user on this machine
//! (`CRED_PERSIST_LOCAL_MACHINE`: it does not roam). Every block calls one `Cred*` function with
//! pointers that live on this stack frame, or frees the single buffer `CredReadW` allocated.
//! The secret bytes are copied out and never logged.

#![allow(unsafe_code)]

use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_NOT_FOUND;
use windows::Win32::Security::Credentials::{
    CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};

use super::SecretStoreError;

fn os(what: &str, error: &windows::core::Error) -> SecretStoreError {
    SecretStoreError::Os(format!("{what}: {error}"))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Frees the buffer of a successful `CredReadW` exactly once, on drop.
struct CredBuffer(*mut CREDENTIALW);

impl Drop for CredBuffer {
    fn drop(&mut self) {
        // SAFETY: the pointer was returned by a successful `CredReadW`, is non-null (checked in
        // `read`), and this guard is its only owner, so it is freed exactly once.
        unsafe { CredFree(self.0.cast_const().cast()) };
    }
}

/// The blob stored under `target`, or `None` if there is no such credential.
pub(super) fn read(target: &str) -> Result<Option<Vec<u8>>, SecretStoreError> {
    let name = wide(target);
    let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated and outlives the call; `raw` is a valid out-pointer on
    // this frame. On success Windows stores a pointer to a buffer we must free with `CredFree`.
    let result = unsafe { CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) };
    if let Err(error) = result {
        if error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0) {
            return Ok(None);
        }
        return Err(os("could not read the credential", &error));
    }
    if raw.is_null() {
        return Err(SecretStoreError::Os(
            "could not read the credential: Windows returned no data".to_owned(),
        ));
    }
    let buffer = CredBuffer(raw);
    // SAFETY: `buffer.0` is the valid, initialised `CREDENTIALW` that `CredReadW` just returned
    // and `buffer` keeps it alive until the end of this function.
    let credential = unsafe { &*buffer.0 };
    let len = credential.CredentialBlobSize as usize;
    if len == 0 || credential.CredentialBlob.is_null() {
        return Ok(Some(Vec::new()));
    }
    // SAFETY: Windows documents `CredentialBlob` as pointing to `CredentialBlobSize` readable
    // bytes inside the buffer owned by `buffer`; the slice is copied before `buffer` drops.
    let blob = unsafe { std::slice::from_raw_parts(credential.CredentialBlob, len) };
    Ok(Some(blob.to_vec()))
}

/// Stores `bytes` as the blob of a generic credential `target`, replacing any previous one.
pub(super) fn write(target: &str, bytes: &[u8]) -> Result<(), SecretStoreError> {
    let mut name = wide(target);
    let mut blob = bytes.to_vec();
    let size = u32::try_from(blob.len())
        .map_err(|_| SecretStoreError::Os("the secret is too large".to_owned()))?;
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        CredentialBlobSize: size,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..CREDENTIALW::default()
    };
    // SAFETY: `credential` is fully initialised; `TargetName` points at the NUL-terminated
    // `name` and `CredentialBlob` at `size` bytes of `blob`, both alive until after the call.
    // `CredWriteW` only reads through these pointers and copies what it keeps.
    let result = unsafe { CredWriteW(&credential, 0) };
    result.map_err(|error| os("could not write the credential", &error))
}

/// Deletes the credential `target`. Tests use it to clean up their own test-only targets.
#[cfg(test)]
pub(super) fn delete(target: &str) -> Result<(), SecretStoreError> {
    use windows::Win32::Security::Credentials::CredDeleteW;
    let name = wide(target);
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) }
        .map_err(|error| os("could not delete the credential", &error))
}
