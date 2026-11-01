//! Tinwave domain types and ids.
//!
//! This crate has no IO, no async runtime and depends on nothing else in the workspace.

#![forbid(unsafe_code)]

mod error;
mod id;
mod tier;
mod time;

pub use error::CoreError;
pub use id::{DeviceId, RecordingId, SessionId, SignalEventId, SourceId};
pub use tier::Tier;
pub use time::{TzOffsetMin, UtcMillis};
