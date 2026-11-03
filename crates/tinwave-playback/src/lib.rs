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
//!   not depend on the signal bus; the [`Controller`] above it owns the poll cadence and maps
//!   those events to T0 signals (and listening sessions) on the `tinwave-signals` bus.
//!
//! The decoder thread resamples tracks whose rate differs from the stream rate (rubato, T-0019);
//! a rate that cannot be converted is reported as a failed track, never played wrong. Integer
//! outputs are dithered where it matters ([`convert_dithered`]).
//!
//! The decoder thread also changes the playback speed, 0.75x to 1.5x with the pitch kept
//! ([`Engine::set_rate`], Signalsmith Stretch, ADR 0006, T-0027): at exactly 1.0 the samples
//! bypass the stretcher; any other rate restarts the stream at the audible position, between
//! 5 ms fades, and the callback only does the fades and the rate bookkeeping ([`RateStats`]).
//! Every change is an [`EventKind::RateChanged`]. Out of scope here: gapless metadata
//! extraction (T-0018).

// `deny`, not `forbid`: the Windows COM calls (`output/windows/os.rs`) and the C++ time stretcher
// (`stretch`) are the only modules that allow `unsafe` (each block carries a SAFETY comment; see Cargo.toml).
#![deny(unsafe_code)]

mod controller;
mod decoder;
mod engine;
mod error;
mod event;
mod output;
mod rate_stage;
mod reader;
mod renderer;
mod resample;
mod shared;
pub mod stretch;
mod types;

pub use controller::{
    BoundedSink, Controller, ControllerConfig, ControllerDiagnostics, ControllerHandle,
    FixedTimezone, LoadIntent, LoadRequest, PlayState, PlaybackStatus, QueueEdit, QueueEditRequest,
    QueueEntry, SinkStats, TimezoneSource, CRITICAL_CAPACITY, DEFAULT_POLL_INTERVAL, MARK_CAPACITY,
};
pub use engine::{Engine, EngineConfig, RateStats};
pub use error::PlaybackError;
pub use event::{EventKind, EventSink, LeaveCause, PlaybackEvent, StartCause};
pub use output::{
    convert_dithered, convert_into, plan_exclusive, resolve_mode, run_render_thread, AudioOutput,
    CpalControl, CpalOutput, DeviceEvent, DeviceFollower, DeviceLost, DeviceSetup,
    DeviceStateChange, Dither, FollowTarget, ManualHandle, ManualOutput, ModeOutcome, OpenRequest,
    OpenState, OutputStream, Pump, PumpStep, RenderDevice, SampleFormat, StreamInfo, StreamMode,
    EXCLUSIVE_CANDIDATES,
};
#[cfg(windows)]
pub use output::{WasapiControl, WasapiOutput};
pub use reader::{FsOpener, SourceOpener};
pub use renderer::Renderer;
pub use types::{EditId, GaplessInfo, Position, QueueItemId, Track};
