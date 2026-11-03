//! Sample-rate conversion for the decoder thread (ADR 0005 decision 5, rubato `Fft`).
//!
//! The decoder thread is the only place audio is resampled: the render callback never sees a
//! file rate. [`StreamResampler`] converts one stream of interleaved stereo `f32` from a file rate
//! to the stream rate and keeps exact book-keeping of the output timeline, which the engine needs
//! for its marks and positions:
//!
//! - **Continuous.** Consecutive tracks of the same rate go through one resampler without a reset,
//!   so there is no filter restart (no phase error, no click) at the boundary.
//! - **Delay-free.** The resampler's start-up delay is trimmed, so output frame `j` is the input at
//!   time `j / ratio`: input frame `n` lands on output frame `round(n * ratio)`, whatever the
//!   chunking. [`StreamResampler::position`] reports that frame for the next track's mark.
//! - **Exact length.** [`StreamResampler::finish`] flushes the tail with silence and cuts the
//!   output to exactly `round(fed * ratio)` frames.
//!
//! # Where time-stretch goes (ADR 0006, T-0027)
//! The chain is `reader -> [stretch] -> [resample] -> ring`. The stretcher works at the file rate
//! and its output is what [`StreamResampler::push`] takes, so it slots in between the reader and
//! the resampler in `Decoder::to_stream_rate`; nothing in this module changes.

use std::fmt::Display;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler, WindowFunction};

use crate::types::CHANNELS;
use crate::PlaybackError;

/// Input frames per resampler call, rounded down to whole FFT blocks (at least one). About 20 ms
/// at 44.1 kHz: small enough that the first audio is not delayed noticeably, large enough that
/// the call overhead is negligible.
const CHUNK_FRAMES: usize = 1024;
/// Target FFT block length, input side. Short blocks keep the delay (half a block of output)
/// under 4 ms; the anti-aliasing filter is as sharp as the block is long.
const FFT_FRAMES: usize = 256;
/// The largest FFT block (input side, frames) accepted. Every standard pair of rates needs under a
/// thousand; a pair with a tiny common divisor (44100 -> 48001) needs a block as long as a second
/// and a delay of half of it, so it is refused instead of played with a long smear.
const MAX_FFT_FRAMES: usize = 16_384;

/// `n * to / from`, rounded to nearest.
pub(crate) fn scale_frames(n: u64, from: u32, to: u32) -> u64 {
    let from = u128::from(from.max(1));
    let scaled = (u128::from(n) * u128::from(to) + from / 2) / from;
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn resampler_error(e: impl Display) -> PlaybackError {
    PlaybackError::Decode(format!("resampler: {e}"))
}

pub(crate) struct StreamResampler {
    inner: Fft<f32>,
    in_rate: u32,
    out_rate: u32,
    /// Stream frame at which output frame 0 of this resampler sits.
    base: u64,
    /// Input frames pushed so far.
    fed: u64,
    /// Output frames handed out so far (after the start-up delay).
    emitted: u64,
    /// Output frames of start-up delay still to drop.
    to_trim: usize,
    /// Input not yet processed: always less than one chunk between calls.
    input: Vec<f32>,
    /// Output of one call.
    block: Vec<f32>,
}

impl std::fmt::Debug for StreamResampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResampler")
            .field("in_rate", &self.in_rate)
            .field("out_rate", &self.out_rate)
            .field("fed", &self.fed)
            .field("emitted", &self.emitted)
            .finish_non_exhaustive()
    }
}

impl StreamResampler {
    /// A resampler from `in_rate` to `out_rate` (stereo), whose output starts at stream frame 0
    /// (see [`start_at`](Self::start_at)).
    ///
    /// # Errors
    /// [`PlaybackError::SampleRateMismatch`] when the pair cannot be converted: a zero rate, or a
    /// pair whose FFT block would be longer than [`MAX_FFT_FRAMES`] (the rates have a tiny common
    /// divisor).
    pub fn new(in_rate: u32, out_rate: u32) -> Result<Self, PlaybackError> {
        let unsupported = PlaybackError::SampleRateMismatch {
            track_rate: in_rate,
            output_rate: out_rate,
        };
        if in_rate == 0 || out_rate == 0 {
            return Err(unsupported);
        }
        // The FFT block is the smallest whole multiple of the rates' reduced ratio that is about
        // 256 frames long, with an even output length: the resampler's delay is half of it, and
        // an odd block would make that half a frame, a phase error the integer delay cannot hold.
        let g = gcd(in_rate, out_rate);
        let (min_in, min_out) = ((in_rate / g) as usize, (out_rate / g) as usize);
        let mut blocks = FFT_FRAMES.div_ceil(min_in).max(1);
        if (blocks * min_out) % 2 == 1 {
            blocks += 1;
        }
        let block = blocks * min_in;
        if block > MAX_FFT_FRAMES {
            return Err(unsupported);
        }
        // Chunks hold whole blocks, so the resampler never keeps input back between calls.
        let per_chunk = (CHUNK_FRAMES / block).max(1);
        let inner = Fft::<f32>::new_custom(
            in_rate as usize,
            out_rate as usize,
            block * per_chunk,
            per_chunk,
            CHANNELS,
            WindowFunction::BlackmanHarris2,
            FixedSync::Input,
        )
        .map_err(|_| unsupported)?;
        let to_trim = inner.output_delay();
        let block = vec![0.0; inner.output_frames_max() * CHANNELS];
        Ok(Self {
            inner,
            in_rate,
            out_rate,
            base: 0,
            fed: 0,
            emitted: 0,
            to_trim,
            input: Vec::with_capacity(2 * CHUNK_FRAMES * CHANNELS),
            block,
        })
    }

    /// Place the start of the output at stream frame `base`.
    #[must_use]
    pub fn start_at(mut self, base: u64) -> Self {
        self.base = base;
        self
    }

    /// The file rate this resampler converts from.
    pub fn in_rate(&self) -> u32 {
        self.in_rate
    }

    /// Output frames the input pushed so far corresponds to.
    fn target(&self) -> u64 {
        scale_frames(self.fed, self.in_rate, self.out_rate)
    }

    /// The stream frame the next input frame lands on: where the next track's mark goes. Counts
    /// frames still inside the resampler, so it is exact whether or not they were emitted yet.
    pub fn position(&self) -> u64 {
        self.base + self.target()
    }

    /// Feed interleaved stereo input and append whatever output is ready to `out`.
    ///
    /// # Errors
    /// The resampler refused a buffer (a bug, not an input condition).
    pub fn push(&mut self, frames: &[f32], out: &mut Vec<f32>) -> Result<(), PlaybackError> {
        debug_assert_eq!(frames.len() % CHANNELS, 0, "whole frames only");
        self.fed += (frames.len() / CHANNELS) as u64;
        self.input.extend_from_slice(frames);
        while self.input.len() >= self.inner.input_frames_next() * CHANNELS {
            self.run(out)?;
        }
        Ok(())
    }

    /// Flush the tail with silence and append it: afterwards exactly `round(fed * ratio)` frames
    /// were emitted in total. Consumes the resampler (the next track of a different rate gets a
    /// new one; after the end of the queue nothing is left to continue).
    ///
    /// # Errors
    /// As [`push`](Self::push).
    pub fn finish(mut self, out: &mut Vec<f32>) -> Result<(), PlaybackError> {
        while self.emitted < self.target() {
            let need = self.inner.input_frames_next() * CHANNELS;
            if self.input.len() < need {
                self.input.resize(need, 0.0);
            }
            self.run(out)?;
        }
        Ok(())
    }

    /// Process one chunk from the front of `input` and append the output.
    fn run(&mut self, out: &mut Vec<f32>) -> Result<(), PlaybackError> {
        let in_frames = self.inner.input_frames_next();
        let out_frames = self.block.len() / CHANNELS;
        let input = InterleavedSlice::new(&self.input[..in_frames * CHANNELS], CHANNELS, in_frames)
            .map_err(resampler_error)?;
        let mut output = InterleavedSlice::new_mut(&mut self.block[..], CHANNELS, out_frames)
            .map_err(resampler_error)?;
        let (consumed, produced) = self
            .inner
            .process_into_buffer(&input, &mut output, None)
            .map_err(resampler_error)?;
        self.input.drain(..consumed * CHANNELS);

        let skip = self.to_trim.min(produced);
        self.to_trim -= skip;
        // Never past the end of what was fed: the silence that flushes the tail stops there.
        let room = self.target().saturating_sub(self.emitted);
        let take = usize::try_from(room).map_or(produced - skip, |r| r.min(produced - skip));
        out.extend_from_slice(&self.block[skip * CHANNELS..(skip + take) * CHANNELS]);
        self.emitted += take as u64;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // test code: a failed expectation is the failure message
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn sine(rate: u32, freq: f64, from: usize, frames: usize) -> Vec<f32> {
        (from..from + frames)
            .flat_map(|n| {
                let s = (0.5 * (2.0 * PI * freq * n as f64 / f64::from(rate)).sin()) as f32;
                [s, -s]
            })
            .collect()
    }

    /// Resample `input` in `step`-frame pushes, finishing at the end.
    fn run_all(in_rate: u32, out_rate: u32, input: &[f32], step: usize) -> Vec<f32> {
        let mut rs = StreamResampler::new(in_rate, out_rate).expect("supported pair");
        let mut out = Vec::new();
        for block in input.chunks(step * CHANNELS) {
            rs.push(block, &mut out).expect("push");
        }
        rs.finish(&mut out).expect("finish");
        out
    }

    /// Largest deviation of the left channel from the ideal sine, skipping `margin` frames at
    /// each end (the filter rings against the silence before and after the clip).
    fn max_error(out: &[f32], rate: u32, freq: f64, margin: usize) -> f64 {
        let frames = out.len() / CHANNELS;
        (margin..frames - margin)
            .map(|j| {
                let ideal = 0.5 * (2.0 * PI * freq * j as f64 / f64::from(rate)).sin();
                (f64::from(out[j * CHANNELS]) - ideal).abs()
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn every_standard_pair_has_exact_length_and_the_right_waveform() {
        for (in_rate, out_rate) in [
            (44_100, 48_000),
            (48_000, 44_100),
            (96_000, 48_000),
            (48_000, 96_000),
            (22_050, 48_000),
            (192_000, 44_100),
            (88_200, 48_000),
            (11_025, 48_000),
        ] {
            let frames = in_rate as usize; // one second
            let input = sine(in_rate, 1_000.0, 0, frames);
            let out = run_all(in_rate, out_rate, &input, 777);
            assert_eq!(
                out.len() / CHANNELS,
                scale_frames(frames as u64, in_rate, out_rate) as usize,
                "{in_rate} -> {out_rate}: length"
            );
            let err = max_error(&out, out_rate, 1_000.0, 2_000);
            assert!(err < 2e-3, "{in_rate} -> {out_rate}: waveform error {err}");
            // The right channel is the inverted left one: channels are not mixed up.
            let (l, r) = (out[2_001 * CHANNELS], out[2_001 * CHANNELS + 1]);
            assert!((l + r).abs() < 2e-3, "{in_rate} -> {out_rate}: channels");
        }
    }

    #[test]
    fn the_output_does_not_depend_on_how_the_input_is_split() {
        let input = sine(44_100, 3_000.0, 0, 30_000);
        let whole = run_all(44_100, 48_000, &input, 30_000);
        for step in [1, 147, 1_023, 1_024, 4_321] {
            assert_eq!(
                run_all(44_100, 48_000, &input, step),
                whole,
                "pushes of {step} frames"
            );
        }
    }

    #[test]
    fn one_continuous_resampler_has_no_error_at_a_track_boundary() {
        // A tone cut in two at an arbitrary frame, as two files of the same album are.
        let (rate, split, total) = (44_100u32, 20_001usize, 40_000usize);
        let all = sine(rate, 5_000.0, 0, total);
        let (a, b) = all.split_at(split * CHANNELS);

        let mut rs = StreamResampler::new(rate, 48_000).expect("pair");
        let mut continuous = Vec::new();
        rs.push(a, &mut continuous).expect("push a");
        let boundary = rs.position();
        rs.push(b, &mut continuous).expect("push b");
        rs.finish(&mut continuous).expect("finish");

        // Independent resamplers: what a reset at every track would do.
        let mut separate = run_all(rate, 48_000, a, 1_024);
        separate.extend(run_all(rate, 48_000, b, 1_024));

        let around = |out: &[f32]| -> f64 {
            let centre = boundary as usize;
            (centre - 300..centre + 300)
                .map(|j| {
                    let ideal = 0.5 * (2.0 * PI * 5_000.0 * j as f64 / 48_000.0).sin();
                    (f64::from(out[j * CHANNELS]) - ideal).abs()
                })
                .fold(0.0, f64::max)
        };
        let err_continuous = around(&continuous);
        let err_separate = around(&separate);
        assert!(err_continuous < 2e-3, "continuous: {err_continuous}");
        assert!(
            err_separate > 10.0 * err_continuous,
            "a reset at the boundary must be visible: {err_separate} vs {err_continuous}"
        );
        assert_eq!(
            boundary,
            scale_frames(split as u64, 44_100, 48_000),
            "the boundary is the scaled input frame"
        );
        assert_eq!(
            continuous.len() / CHANNELS,
            scale_frames(total as u64, 44_100, 48_000) as usize
        );
    }

    #[test]
    fn position_is_the_scaled_input_count_before_any_output_exists() {
        let mut rs = StreamResampler::new(44_100, 48_000)
            .expect("pair")
            .start_at(1_000);
        assert_eq!(rs.position(), 1_000);
        let mut out = Vec::new();
        // Less than one chunk: nothing comes out, but the position already moved.
        rs.push(&sine(44_100, 440.0, 0, 100), &mut out)
            .expect("push");
        assert!(out.is_empty());
        assert_eq!(rs.position(), 1_000 + scale_frames(100, 44_100, 48_000));
    }

    #[test]
    fn a_clip_shorter_than_the_delay_still_comes_out_whole() {
        for frames in [0usize, 1, 10, 147, 2_000] {
            let input = sine(44_100, 440.0, 0, frames);
            let out = run_all(44_100, 48_000, &input, 64);
            assert_eq!(
                out.len() / CHANNELS,
                scale_frames(frames as u64, 44_100, 48_000) as usize,
                "{frames} frames"
            );
        }
    }

    #[test]
    fn a_pair_that_needs_a_huge_block_is_refused() {
        let err = StreamResampler::new(44_100, 48_001).expect_err("refused");
        assert_eq!(
            err,
            PlaybackError::SampleRateMismatch {
                track_rate: 44_100,
                output_rate: 48_001
            }
        );
        assert!(StreamResampler::new(0, 48_000).is_err());
    }

    #[test]
    fn scaling_rounds_to_nearest() {
        assert_eq!(scale_frames(44_100, 44_100, 48_000), 48_000);
        assert_eq!(scale_frames(1, 44_100, 48_000), 1);
        assert_eq!(scale_frames(3, 96_000, 48_000), 2); // 1.5 -> 2
        assert_eq!(scale_frames(5, 1, 1), 5);
    }
}
