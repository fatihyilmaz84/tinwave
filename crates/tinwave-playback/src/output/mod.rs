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

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::{PlaybackError, Renderer};

/// What the engine asks of an output when opening a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenRequest<'a> {
    /// Output device id; `None` is the system default.
    pub device: Option<&'a str>,
    /// Stream sample rate, stereo `f32`. In shared mode this is the engine's configured rate and
    /// the engine resamples files of other rates to it (decoder thread, T-0019), so the callback
    /// only ever sees this rate; a backend that has to convert to the device mix rate does so
    /// itself. When [`AudioOutput::wants_exclusive`] is true the engine asks for the rate of the
    /// file that is about to play (T-0057): a rate the device does not open exclusively is a
    /// refusal, and the engine opens again at its configured rate.
    pub sample_rate: u32,
    /// Open exclusively or not at all (T-0066). The engine sets it for a rate that is not its
    /// own, where a shared stream would be thrown away at once: an output with an exclusive mode
    /// then returns an error instead of falling back to shared, so a refusal costs no shared
    /// open. It is false for the engine's own rate, where shared is the right fallback.
    /// The error for a refusal is [`PlaybackError::ExclusiveRefused`]; any other error is a
    /// failure of the device and ends the open as it always did. Outputs that are not in exclusive mode ignore
    /// the flag; an output that ignores it altogether still works, the engine also recognises a
    /// shared stream as the refusal.
    pub exclusive_only: bool,
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

    /// Whether the next stream is to be opened in exclusive mode. The engine then follows the
    /// rate of each file (reopening the stream at a track whose rate differs, ADR 0005 decision
    /// 5) instead of resampling to one rate. Streams report what they really got in
    /// [`OutputStream::info`]. Outputs without an exclusive mode keep the default, `false`.
    fn wants_exclusive(&self) -> bool {
        false
    }
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
    exclusive: AtomicBool,
    /// Exclusive-only opens that failed because the fake device refused the rate.
    refused: AtomicUsize,
    /// Rates the fake device accepts in exclusive mode; `None` accepts every rate.
    device_rates: Mutex<Option<Vec<u32>>>,
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
        let info = self.stream_info(request);
        if request.exclusive_only && self.wants_exclusive() && info.mode != StreamMode::Exclusive {
            // As the WASAPI output does: no shared stream in place of the refused exclusive one.
            // The open is counted (the device was asked), nothing is left open.
            self.shared.refused.fetch_add(1, Ordering::AcqRel);
            return Err(PlaybackError::ExclusiveRefused(format!(
                "the device does not accept {} Hz",
                request.sample_rate
            )));
        }
        self.shared.lost.store(false, Ordering::Release);
        *lock(&self.shared.renderer) = Some(renderer);
        Ok(Box::new(ManualStream {
            shared: Arc::clone(&self.shared),
            info,
        }))
    }

    fn wants_exclusive(&self) -> bool {
        self.shared.exclusive.load(Ordering::Acquire)
    }
}

impl ManualOutput {
    /// What a stream opened for `request` looks like: unknown (shared) unless exclusive mode was
    /// asked for, then exclusive at a rate the fake device accepts and, for any other, shared
    /// with the reason, as a real endpoint that refuses a rate does.
    fn stream_info(&self, request: &OpenRequest<'_>) -> StreamInfo {
        if !self.wants_exclusive() {
            return StreamInfo::unknown();
        }
        let accepted = lock(&self.shared.device_rates)
            .as_ref()
            .is_none_or(|rates| rates.contains(&request.sample_rate));
        let mut info = StreamInfo::unknown();
        info.backend = "manual";
        if accepted {
            info.mode = StreamMode::Exclusive;
            info.format = SampleFormat::I24In32;
        } else {
            info.fallback = Some(format!(
                "exclusive mode refused: the device does not accept {} Hz",
                request.sample_rate
            ));
        }
        info
    }
}

struct ManualStream {
    shared: Arc<ManualShared>,
    info: StreamInfo,
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

    fn info(&self) -> StreamInfo {
        self.info.clone()
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

    /// Ask for exclusive mode (`true`) or shared mode (`false`) from the next `open` on, as
    /// `WasapiControl::set_exclusive` does. While it is on, streams report
    /// [`StreamMode::Exclusive`] at the rates [`set_device_rates`](Self::set_device_rates) allows
    /// and shared mode with a [`StreamInfo::fallback`] reason at any other. The open stream is
    /// not touched.
    pub fn set_exclusive(&self, exclusive: bool) {
        self.shared.exclusive.store(exclusive, Ordering::Release);
    }

    /// The sample rates the fake device accepts in exclusive mode (an empty list accepts none).
    /// Every rate is accepted until this is called.
    pub fn set_device_rates(&self, rates: &[u32]) {
        *lock(&self.shared.device_rates) = Some(rates.to_vec());
    }

    /// How many exclusive-only opens failed because the fake device does not take the rate. Each is
    /// also in [`opens`](Self::opens); the others in it produced a stream.
    #[must_use]
    pub fn refused_opens(&self) -> usize {
        self.shared.refused.load(Ordering::Acquire)
    }

    /// Every `(device, sample_rate)` the output was opened with, in order, including opens the
    /// device then refused (as a shared stream, or as an error for an exclusive-only request).
    #[must_use]
    pub fn opens(&self) -> Vec<(Option<String>, u32)> {
        lock(&self.shared.opens).clone()
    }
}
