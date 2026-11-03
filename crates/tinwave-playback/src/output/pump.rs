//! The device-independent half of an event-driven audio callback.
//!
//! An event-driven output (WASAPI) wakes when the device wants data, asks how much room there is,
//! and writes. [`Pump`] does everything in that wake-up except the device calls, which the output
//! supplies through [`RenderDevice`]: render from the ring, convert to the device format, write.
//! Because the device is a trait, the callback can be driven by a fake in tests, including the
//! counting-allocator proof that it never touches the heap.
//!
//! # Real-time contract
//! [`Pump::feed`] and [`Pump::step`] allocate nothing: both scratch buffers are sized in
//! [`Pump::new`] (the dither generator is one `u64`), and the `RenderDevice` implementation must not allocate in `wait`,
//! `available_frames`, `write` or `is_alive` either.

use super::convert::{convert_dithered, Dither, SampleFormat};
use crate::types::CHANNELS;
use crate::Renderer;

/// The device is gone or unusable; the stream must be re-opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLost;

/// The device calls an event-driven callback needs.
pub trait RenderDevice {
    /// Wait until the device wants data. `Ok(true)` when it does, `Ok(false)` on timeout.
    ///
    /// # Errors
    /// [`DeviceLost`] when waiting itself failed.
    fn wait(&mut self, timeout_ms: u32) -> Result<bool, DeviceLost>;

    /// Frames of free room in the device buffer.
    ///
    /// # Errors
    /// [`DeviceLost`] when the device cannot be queried (removed, invalidated).
    fn available_frames(&mut self) -> Result<usize, DeviceLost>;

    /// Write `frames` frames of `bytes` to the device.
    ///
    /// # Errors
    /// [`DeviceLost`] when the write failed.
    fn write(&mut self, frames: usize, bytes: &[u8]) -> Result<(), DeviceLost>;

    /// Start the device after the first buffer was written.
    ///
    /// # Errors
    /// [`DeviceLost`] when the device would not start.
    fn start(&mut self) -> Result<(), DeviceLost>;

    /// Stop the device. Best effort: the stream is ending anyway.
    fn stop(&mut self);

    /// Asked after a wait timed out: is the device still there? (A removed device stops
    /// signalling its event, so a timeout is the only symptom.)
    fn is_alive(&mut self) -> bool;
}

/// What one wake-up did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpStep {
    /// `frames` frames were written, `real` of them carry audio (the rest is silence).
    Fed {
        /// Frames written to the device.
        frames: usize,
        /// Frames of real audio among them.
        real: usize,
    },
    /// Nothing to do (timeout with a live device, or no room in the buffer).
    Idle,
    /// The device is gone.
    Lost,
}

/// Renders, converts and writes. Owns the [`Renderer`] and gives it back with
/// [`into_renderer`](Self::into_renderer) so the thread that finishes with the stream, not the
/// callback thread, decides where it is dropped.
#[derive(Debug)]
pub struct Pump {
    renderer: Renderer,
    format: SampleFormat,
    buffer_frames: usize,
    stream_latency_frames: u64,
    scratch: Vec<f32>,
    bytes: Vec<u8>,
    dither: Dither,
}

impl Pump {
    /// A pump for a device buffer of `buffer_frames` frames in `format`. `stream_latency_frames`
    /// is the OS-reported latency beyond the buffer.
    #[must_use]
    pub fn new(
        renderer: Renderer,
        format: SampleFormat,
        buffer_frames: usize,
        stream_latency_frames: u64,
    ) -> Self {
        Self {
            renderer,
            format,
            buffer_frames,
            stream_latency_frames,
            scratch: vec![0.0; buffer_frames * CHANNELS],
            bytes: vec![0; buffer_frames * CHANNELS * format.bytes_per_sample()],
            dither: Dither::new(Dither::DEFAULT_SEED),
        }
    }

    /// Use `dither` (another seed) for integer formats; only tests need this.
    #[must_use]
    pub fn with_dither(mut self, dither: Dither) -> Self {
        self.dither = dither;
        self
    }

    /// Wait for the device, then fill whatever room it has.
    pub fn step<D: RenderDevice>(&mut self, device: &mut D, timeout_ms: u32) -> PumpStep {
        match device.wait(timeout_ms) {
            Ok(true) => self.feed(device),
            Ok(false) if device.is_alive() => PumpStep::Idle,
            Ok(false) | Err(DeviceLost) => PumpStep::Lost,
        }
    }

    /// Fill the room the device has now, without waiting. Also used once before the stream
    /// starts, so the first callback already carries audio.
    pub fn feed<D: RenderDevice>(&mut self, device: &mut D) -> PumpStep {
        let Ok(available) = device.available_frames() else {
            return PumpStep::Lost;
        };
        let frames = available.min(self.buffer_frames);
        if frames == 0 {
            return PumpStep::Idle;
        }
        // The block's last frame is heard after everything already queued, this block, and the OS
        // stream latency. Filling all the room leaves the buffer full, so that is one buffer.
        let latency = self.buffer_frames as u64 + self.stream_latency_frames;
        let out = &mut self.scratch[..frames * CHANNELS];
        let real = self.renderer.render(out, latency);
        let width = CHANNELS * self.format.bytes_per_sample();
        let dst = &mut self.bytes[..frames * width];
        convert_dithered(self.format, out, dst, &mut self.dither);
        match device.write(frames, dst) {
            Ok(()) => PumpStep::Fed { frames, real },
            Err(DeviceLost) => PumpStep::Lost,
        }
    }

    /// The renderer, for a host that must service the seek handshake while the device is not
    /// asking for data (an empty `render` call acknowledges a flush without consuming audio).
    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    /// Give the renderer back (to be dropped by the thread that owns the stream).
    #[must_use]
    pub fn into_renderer(self) -> Renderer {
        self.renderer
    }
}
