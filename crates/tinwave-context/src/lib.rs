//! Tinwave context providers: what the listener's situation is right now, cheaply and offline.
//!
//! All OS code for context lives in this crate (`docs/rules/architecture.md`). Providers cache
//! their value and `snapshot()` only reads the cache; they never block playback.
//!
//! Today this crate has the audio providers: [`audio::AudioMonitor`] with the output-device
//! (T1) and system-volume (T0) views, built on Windows MMDevice notifications (ADR 0004).
//! Other OSes report [`Capability::Unsupported`] until their own tickets land.
//!
//! It also owns the OS keychain access for the `DeviceSecret` ([`secret_store`]): Windows
//! Credential Manager today, `Unsupported` elsewhere.
//!
//! `unsafe` is denied crate-wide; the only modules allowed to use it are the Windows COM module
//! (`audio/win/com.rs`) and the Credential Manager module (`secret_store/win.rs`). Both are small
//! and document every block.

#![deny(unsafe_code)]

pub mod audio;
mod error;
mod provider;
pub mod secret_store;

pub use error::ContextError;
pub use provider::{
    Capability, ContextFields, ContextProvider, OutputDeviceFields, ProviderId, SystemVolumeFields,
};
pub use secret_store::{OsSecretStore, SecretStore, SecretStoreError};
