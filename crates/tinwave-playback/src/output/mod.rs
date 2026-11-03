//! The seam between the engine core and an operating-system audio API, and the outputs behind it.
//!
//! - [`AudioOutput`] / [`OutputStream`]: the contract (T-0016).
//! - `WasapiOutput` (Windows only): direct WASAPI, shared by default, exclusive on request,
//!   event driven, MMCSS "Pro Audio". The only module with `unsafe` is `windows/os.rs`.
//! - [`CpalOutput`]: generic output for macOS and Linux, and the Windows fallback.
//! - [`ManualOutput`]: no device, no thread; the caller plays the audio callback (tests).
//! - [`Pump`] / [`RenderDevice`]: the device-independent half of an event-driven callback
//!   (render, convert, write), so it can be tested with a fake device.
//!
//! None of the outputs ever touches endpoint volume or mute: in-app volume is the renderer's
//! software gain (ADR 0004 decision 5). Device and system-volume *signals* come from
//! `tinwave-context`; an output only reports `is_lost` so the host can `switch_device`.

mod convert;
mod cpal;
mod follow;
mod info;
mod plan;
mod pump;
mod stream_thread;
#[cfg(windows)]
mod windows;

pub use convert::{convert_dithered, convert_into, Dither, SampleFormat};
pub use cpal::{CpalControl, CpalOutput};
pub use follow::{DeviceEvent, DeviceFollower, DeviceStateChange, FollowTarget};
pub use info::{StreamInfo, StreamMode};
pub use plan::{plan_exclusive, resolve_mode, ModeOutcome, EXCLUSIVE_CANDIDATES};
pub use pump::{DeviceLost, Pump, PumpStep, RenderDevice};
pub use stream_thread::{run_render_thread, DeviceSetup, OpenState};
#[cfg(windows)]
pub use windows::{WasapiControl, WasapiOutput};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::{PlaybackError, Renderer};

/// What the engine asks of an output when opening a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenRequest<'a> {
    /// Output device id; `None` is the system default.
    pub device: Option<&'a str>,
    /// Stream sample rate, stereo `f32`. The engine resamples files of other rates to it (decoder
    /// thread, T-0019), so the callback only ever sees this rate; a backend that has to convert to
    /// the device mix rate does so itself.
    pub sample_rate: u32,
}

/// An audio backend: WASAPI, cpal, or the in-memory [`ManualOutput`].
pub trait AudioOutput: Send {
    /// Open a stream that calls `renderer.render` from its audio callback.
    ///
    /// # Errors
    /// [`PlaybackError::Output`] when the device or format is unavailable.
    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError>;
}

/// A running output stream. **Dropping it stops the stream, and when `drop` returns the audio
/// callback is guaranteed not to be running and never to run again** (the engine relies on this
/// to hand a new ring to a new stream without a data race on the shared counters).
pub trait OutputStream: Send {
    /// The device was removed or the stream invalidated; it must be re-opened.
    fn is_lost(&self) -> bool;

    /// How the stream was opened (backend, mode, format, latency, and why exclusive mode was not
    /// used, if it was asked for). Backends that do not know return [`StreamInfo::unknown`].
    fn info(&self) -> StreamInfo {
        StreamInfo::unknown()
    }
}

/// An output without a device or a thread: the caller plays the part of the audio callback by
/// calling [`ManualHandle::render`]. Deterministic, silent, used by every engine test and usable
/// by any headless consumer.
#[derive(Debug)]
pub struct ManualOutput {
    shared: Arc<ManualShared>,
}

/// The caller's side of a [`ManualOutput`]: renders blocks on the calling thread.
#[derive(Debug, Clone)]
pub struct ManualHandle {
    shared: Arc<ManualShared>,
}

#[derive(Debug, Default)]
struct ManualShared {
    renderer: Mutex<Option<Renderer>>,
    lost: AtomicBool,
    opens: Mutex<Vec<(Option<String>, u32)>>,
    fail_next_open: AtomicBool,
}

impl ManualOutput {
    /// A new output and the handle that drives it.
    #[must_use]
    pub fn new() -> (Self, ManualHandle) {
        let shared = Arc::new(ManualShared::default());
        (
            Self {
                shared: Arc::clone(&shared),
            },
            ManualHandle { shared },
        )
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AudioOutput for ManualOutput {
    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError> {
        if self.shared.fail_next_open.swap(false, Ordering::AcqRel) {
            return Err(PlaybackError::Output("manual output: open refused".into()));
        }
        lock(&self.shared.opens).push((request.device.map(str::to_owned), request.sample_rate));
        self.shared.lost.store(false, Ordering::Release);
        *lock(&self.shared.renderer) = Some(renderer);
        Ok(Box::new(ManualStream {
            shared: Arc::clone(&self.shared),
        }))
    }
}

struct ManualStream {
    shared: Arc<ManualShared>,
}

impl std::fmt::Debug for ManualStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManualStream").finish_non_exhaustive()
    }
}

impl OutputStream for ManualStream {
    fn is_lost(&self) -> bool {
        self.shared.lost.load(Ordering::Acquire)
    }
}

impl Drop for ManualStream {
    fn drop(&mut self) {
        lock(&self.shared.renderer).take();
    }
}

impl ManualHandle {
    /// Run one audio callback on this thread. Returns frames of real audio, or `None` when no
    /// stream is open.
    pub fn render(&self, out: &mut [f32], latency_frames: u64) -> Option<usize> {
        lock(&self.shared.renderer)
            .as_mut()
            .map(|r| r.render(out, latency_frames))
    }

    /// Decoded frames waiting in the open stream's ring, or `None` when no stream is open.
    #[must_use]
    pub fn buffered_frames(&self) -> Option<usize> {
        lock(&self.shared.renderer)
            .as_ref()
            .map(Renderer::buffered_frames)
    }

    /// Whether a stream is currently open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        lock(&self.shared.renderer).is_some()
    }

    /// Make the open stream report [`OutputStream::is_lost`].
    pub fn set_lost(&self, lost: bool) {
        self.shared.lost.store(lost, Ordering::Release);
    }

    /// Make the next `open` fail once.
    pub fn fail_next_open(&self) {
        self.shared.fail_next_open.store(true, Ordering::Release);
    }

    /// Every `(device, sample_rate)` the output was opened with, in order.
    #[must_use]
    pub fn opens(&self) -> Vec<(Option<String>, u32)> {
        lock(&self.shared.opens).clone()
    }
}
