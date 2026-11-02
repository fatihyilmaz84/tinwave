//! Tinwave context providers: what the listener's situation is right now, cheaply and offline.
//!
//! All OS code for context lives in this crate (`docs/rules/architecture.md`). Providers cache
//! their value and `snapshot()` only reads the cache; they never block playback.
//!
//! Today this crate has the audio providers: [`audio::AudioMonitor`] with the output-device
//! (T1) and system-volume (T0) views, built on Windows MMDevice notifications (ADR 0004).
//! Other OSes report [`Capability::Unsupported`] until their own tickets land.
//!
//! `unsafe` is denied crate-wide; the only module allowed to use it is the Windows COM module
//! (`audio/win/com.rs`), which is small and documents every block.

#![deny(unsafe_code)]

pub mod audio;
mod error;
mod provider;

pub use error::ContextError;
pub use provider::{
    Capability, ContextFields, ContextProvider, OutputDeviceFields, ProviderId, SystemVolumeFields,
};
