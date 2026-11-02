//! Tinwave domain types and ids.
//!
//! This crate has no IO, no async runtime and depends on nothing else in the workspace.

#![forbid(unsafe_code)]

mod clock;
mod error;
mod id;
mod rng;
mod tier;
mod time;

pub use clock::{Clock, ManualClock, SystemClock};
pub use error::CoreError;
pub use id::{DeviceId, IdGenerator, RecordingId, SessionId, SignalEventId, SourceId};
pub use rng::{CryptoRng, Rng, SeededRng, SystemRng};
pub use tier::Tier;
pub use time::{TzOffsetMin, UtcMillis};
