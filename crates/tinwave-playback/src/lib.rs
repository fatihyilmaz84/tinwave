//! Tinwave playback engine core (spec section 7, ADR 0005).
//!
//! ```text
//! control thread --commands--> decoder thread --f32 ring (rtrb, 1 s)--> Renderer (audio callback)
//!       ^                       (Symphonia, only      lock-free SPSC       |  atomics only
//!       +------ events <------- thread touching files)                     v
//!                                                                    AudioOutput (T-0017)
//! ```
//!
//! - [`Engine`] is the control handle: load, seek, pause, volume, device switch, [`Engine::poll`].
//! - [`Renderer`] runs inside the OS audio callback and never allocates, locks or does IO.
//! - [`AudioOutput`] is what an OS backend implements: `WasapiOutput` (Windows, shared and
//!   exclusive), [`CpalOutput`] (macOS, Linux, Windows fallback), and [`ManualOutput`], the
//!   deterministic in-memory one used by tests and headless callers.
//! - The engine reports [`PlaybackEvent`]s to an [`EventSink`]. It constructs no signals and does
//!   not depend on the signal bus; the layer above maps events to T0 signals.
//!
//! Out of scope here: OS outputs (T-0017), resampling and dither (T-0019), gapless metadata
//! extraction (T-0018), time-stretch (T-0009). Tracks must have the stream sample rate; anything
//! else is reported as a failed track, never played wrong.

// `deny`, not `forbid`: the Windows COM calls live in `output/windows/os.rs`, the only module that
// allows `unsafe` (each block carries a SAFETY comment; see Cargo.toml).
#![deny(unsafe_code)]

mod decoder;
mod engine;
mod error;
mod event;
mod output;
mod reader;
mod renderer;
mod shared;
mod types;

pub use engine::{Engine, EngineConfig};
pub use error::PlaybackError;
pub use event::{EventKind, EventSink, LeaveCause, PlaybackEvent, StartCause};
pub use output::{
    convert_into, plan_exclusive, resolve_mode, AudioOutput, CpalControl, CpalOutput, DeviceEvent,
    DeviceFollower, DeviceLost, DeviceStateChange, FollowTarget, ManualHandle, ManualOutput,
    ModeOutcome, OpenRequest, OutputStream, Pump, PumpStep, RenderDevice, SampleFormat, StreamInfo,
    StreamMode, EXCLUSIVE_CANDIDATES,
};
#[cfg(windows)]
pub use output::{WasapiControl, WasapiOutput};
pub use reader::{FsOpener, SourceOpener};
pub use renderer::Renderer;
pub use types::{GaplessInfo, Position, QueueItemId, Track};
