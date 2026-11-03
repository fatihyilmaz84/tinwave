//! Pitch-preserving time stretch: a safe wrapper over Signalsmith Stretch (T-0026, ADR 0006).
//!
//! The algorithm is the MIT-licensed C++ header library pinned in `vendor/signalsmith/`
//! (see its README for the exact version). `build.rs` compiles `tw_stretch.cpp`, a small C ABI
//! over it, with the `cc` crate; nothing here needs bindgen or libclang.
//!
//! This is the only module of the crate's library, besides the Windows output's `os.rs`, that
//! allows `unsafe`: the calls into that C ABI. The engine drives it from the decoder thread
//! (`rate_stage.rs`, T-0027).
//!
//! # How it is meant to be driven
//!
//! One [`Stretcher`] per stream, on the decoder thread, never inside the audio callback. Each
//! [`Stretcher::process`] call takes `in_frames` interleaved source frames and fills `out_frames`
//! output frames; the playback rate is `in_frames / out_frames` (1.25 consumes 5 source frames
//! for every 4 frames produced). The caller owns the arithmetic that keeps the source position
//! exact: a fixed `out_frames` per call, `floor(out_frames * rate + carry)` input frames, and the
//! remainder carried to the next call ([`BlockPlan`] does exactly this, ADR 0006 decision 3).
//! The useful range is [`MIN_RATE`] to [`MAX_RATE`]; rate exactly 1.0 is bypassed by the engine,
//! not here.
//!
//! The stretcher delays its output: after a [`Stretcher::reset`] the first
//! [`Stretcher::discard_frames`] output frames are the lead-in and must be dropped, so that the
//! first frame kept is the audio at the source position the stretcher was fed from.
//!
//! # Real-time properties
//!
//! After [`Stretcher::new`] returns, [`Stretcher::process`], [`Stretcher::flush`] and
//! [`Stretcher::reset`] allocate nothing, take no lock, make no system call and never block;
//! they fail with an error rather than throw. The tests prove it for the Rust and the C++ heap.
//! The output is deterministic: the same input gives bit-identical output (the C++ random
//! engine, used only for rates below 0.5, is seeded with a constant).
#![allow(unsafe_code)]

mod ffi;
mod plan;

use std::ptr::NonNull;

pub use plan::{BlockPlan, BLOCK_FRAMES};

/// Slowest playback rate the engine accepts (source frames per output frame).
pub const MIN_RATE: f64 = 0.75;
/// Fastest playback rate the engine accepts.
pub const MAX_RATE: f64 = 1.5;

/// Most channels a [`Stretcher`] is configured for.
pub const MAX_CHANNELS: usize = 8;
/// Lowest supported sample rate, in Hz.
pub const MIN_SAMPLE_RATE: u32 = 8_000;
/// Highest supported sample rate, in Hz.
pub const MAX_SAMPLE_RATE: u32 = 384_000;

/// Why a [`Stretcher`] could not be created or a call was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StretchError {
    /// `new` was given a channel count or sample rate outside the supported ranges, or the
    /// memory for the stretcher could not be allocated.
    #[error(
        "cannot configure a stretcher for {channels} channels at {sample_rate} Hz \
         (1 to {MAX_CHANNELS} channels, {MIN_SAMPLE_RATE} to {MAX_SAMPLE_RATE} Hz) or out of memory"
    )]
    Unsupported {
        /// The requested channel count.
        channels: usize,
        /// The requested sample rate.
        sample_rate: u32,
    },
    /// A buffer is not a whole number of interleaved frames.
    #[error("a buffer of {len} samples is not a whole number of {channels}-channel frames")]
    NotWholeFrames {
        /// The buffer length in samples.
        len: usize,
        /// The stretcher's channel count.
        channels: usize,
    },
    /// A buffer holds more frames than one call accepts.
    #[error("{frames} frames in one call is more than the stretcher accepts")]
    TooLong {
        /// The offending frame count.
        frames: usize,
    },
    /// The playback rate for [`Stretcher::discard_frames`] is not a positive, finite number.
    #[error("playback rate {rate} is not a positive finite number")]
    BadRate {
        /// The rejected rate.
        rate: f64,
    },
    /// The C++ side reported a failure it could not recover from (an internal exception).
    #[error("the stretcher failed internally (code {code})")]
    Internal {
        /// The code the C ABI returned.
        code: i32,
    },
}

/// A time stretcher for interleaved `f32` audio. See the [module documentation](self).
///
/// `Send` (the decoder thread owns it) but not `Sync`: every call takes `&mut self`.
#[derive(Debug)]
pub struct Stretcher {
    handle: NonNull<ffi::TwStretch>,
    channels: usize,
    sample_rate: u32,
}

// SAFETY: the handle is owned exclusively by this value and every call that touches it needs
// `&mut self` or `&self` on one thread at a time. The C++ object keeps no thread-local state and
// no pointers to its creator's stack, so it may be used from whichever thread owns the value.
unsafe impl Send for Stretcher {}

impl Stretcher {
    /// Creates a stretcher with the default preset (a 120 ms analysis block, 30 ms interval) for
    /// `channels` interleaved channels at `sample_rate` Hz. This is the only call that allocates
    /// (about 1 MB at 44.1 kHz stereo).
    ///
    /// # Errors
    /// [`StretchError::Unsupported`] for `channels` outside `1..=MAX_CHANNELS`, a sample rate
    /// outside `MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE`, or when memory is exhausted.
    pub fn new(channels: usize, sample_rate: u32) -> Result<Self, StretchError> {
        let unsupported = || StretchError::Unsupported {
            channels,
            sample_rate,
        };
        let c_channels = i32::try_from(channels).map_err(|_| unsupported())?;
        // SAFETY: plain by-value arguments; the C function validates them and returns null for
        // anything it cannot build.
        let raw = unsafe { ffi::tw_stretch_create(c_channels, sample_rate as f32) };
        NonNull::new(raw)
            .map(|handle| Self {
                handle,
                channels,
                sample_rate,
            })
            .ok_or_else(unsupported)
    }

    /// Interleaved channels per frame.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The sample rate the stretcher was configured for.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Frames between a source frame going in and the centre of its effect.
    pub fn input_latency_frames(&self) -> usize {
        // SAFETY: `handle` is a live object owned by `self`.
        unsafe { ffi::tw_stretch_input_latency(self.handle.as_ptr()) }
    }

    /// Frames between the centre of the effect and the output it produces.
    pub fn output_latency_frames(&self) -> usize {
        // SAFETY: `handle` is a live object owned by `self`.
        unsafe { ffi::tw_stretch_output_latency(self.handle.as_ptr()) }
    }

    /// Output frames to drop after a [`reset`](Self::reset) (or creation) before the first frame
    /// that corresponds to the source position the stretcher was fed from, when running at
    /// `rate` (source frames per output frame): `out_latency + in_latency / rate`, rounded
    /// (ADR 0006 decision 4). Checked against a sine in `tests/stretch.rs`.
    ///
    /// # Errors
    /// [`StretchError::BadRate`] unless `rate` is positive and finite.
    pub fn discard_frames(&self, rate: f64) -> Result<usize, StretchError> {
        if !(rate.is_finite() && rate > 0.0) {
            return Err(StretchError::BadRate { rate });
        }
        let lag = self.output_latency_frames() as f64 + self.input_latency_frames() as f64 / rate;
        // `as` saturates; a lag beyond usize::MAX is not a real configuration.
        Ok(lag.round() as usize)
    }

    /// Forgets all history: the next [`process`](Self::process) starts a fresh stream. Does not
    /// allocate.
    pub fn reset(&mut self) {
        // SAFETY: `handle` is a live object owned by `self`, and `&mut self` excludes other users.
        unsafe { ffi::tw_stretch_reset(self.handle.as_ptr()) }
    }

    /// Consumes all frames of `input` and fills all of `output` (both interleaved with
    /// [`channels`](Self::channels) channels). The playback rate is `input` frames over `output`
    /// frames. Either may be empty: empty `input` makes silence-fed output, empty `output` just
    /// stores the input as history. Allocates nothing.
    ///
    /// # Errors
    /// [`StretchError::NotWholeFrames`] or [`StretchError::TooLong`] for an unusable buffer
    /// (`output` is left untouched), [`StretchError::Internal`] if the C++ side threw.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), StretchError> {
        let in_frames = self.frames_of(input.len())?;
        let out_frames = self.frames_of(output.len())?;
        // SAFETY: `handle` is live and exclusively borrowed. The pointers come from live slices
        // that hold exactly `in_frames * channels` and `out_frames * channels` floats, the input
        // is only read and the output only written, and the two cannot overlap (a `&` and a
        // `&mut` slice never alias). The C side does not keep either pointer.
        let code = unsafe {
            ffi::tw_stretch_process(
                self.handle.as_ptr(),
                input.as_ptr(),
                in_frames,
                output.as_mut_ptr(),
                out_frames,
            )
        };
        Self::check(code)
    }

    /// Reads the tail still held inside the stretcher, with no further input, into `output`
    /// (zero-filled past the tail). Use [`output_latency_frames`](Self::output_latency_frames)
    /// frames or more to get all of it. Call [`reset`](Self::reset) before feeding the stretcher
    /// a new stream. Allocates nothing.
    ///
    /// # Errors
    /// As [`process`](Self::process).
    pub fn flush(&mut self, output: &mut [f32]) -> Result<(), StretchError> {
        let out_frames = self.frames_of(output.len())?;
        // SAFETY: `handle` is live and exclusively borrowed; `output` is a live slice of exactly
        // `out_frames * channels` floats that the C side only writes and does not keep.
        let code =
            unsafe { ffi::tw_stretch_flush(self.handle.as_ptr(), output.as_mut_ptr(), out_frames) };
        Self::check(code)
    }

    /// Frames in a buffer of `len` samples, if that is a whole number the C ABI accepts.
    fn frames_of(&self, len: usize) -> Result<usize, StretchError> {
        if !len.is_multiple_of(self.channels) {
            return Err(StretchError::NotWholeFrames {
                len,
                channels: self.channels,
            });
        }
        let frames = len / self.channels;
        if i32::try_from(frames).is_err() {
            return Err(StretchError::TooLong { frames });
        }
        Ok(frames)
    }

    fn check(code: i32) -> Result<(), StretchError> {
        match code {
            0 => Ok(()),
            code => Err(StretchError::Internal { code }),
        }
    }
}

impl Drop for Stretcher {
    fn drop(&mut self) {
        // SAFETY: `handle` came from `tw_stretch_create`, is destroyed exactly once, here, and
        // no reference to it outlives `self`.
        unsafe { ffi::tw_stretch_destroy(self.handle.as_ptr()) }
    }
}
