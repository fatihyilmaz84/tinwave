//! The time-stretch stage of the decoder thread (ADR 0006): `reader -> [stretch] -> resample -> ring`.
//!
//! A [`StretchStage`] turns the packets of one track, read at the file's rate, into audio that
//! plays `rate` times as fast without changing pitch. It is driven the way ADR 0006 decision 3
//! prescribes ([`BlockPlan`]): a fixed [`BLOCK_FRAMES`] of output per call and
//! `floor(out * rate + carry)` frames of input, so the source position is exact by construction.
//!
//! One stage serves one track at a time: [`begin`](StretchStage::begin) arms it for the frame
//! the reader was seeked to, [`push`](StretchStage::push) takes the decoded packets,
//! [`finish`](StretchStage::finish) runs the tail out at the end of the track. After `begin` the
//! first [`Stretcher::discard_frames`] frames of output are the lead-in and are dropped, so the
//! first frame that leaves the stage is the audio at the frame the stream starts from; the
//! restart after a rate change costs about a millisecond of CPU for that, far faster than real
//! time.
//!
//! # Pre-roll
//! A stretcher that starts cold at the position it is meant to play from has half an analysis
//! window of silence behind it, and its first 60 ms or so overshoot (a steady sine comes out up
//! to 5 % too loud and decays to its true level). So the reader is seeked a little before the
//! start ([`preroll_frames`](StretchStage::preroll_frames) source frames, fewer at the very
//! start of a track), and `begin` is told how many of the frames it will be given are such
//! history: they are stretched like the rest, and the output they produce is dropped with the
//! lead-in.
//!
//! The stage's output is **exact in length**: a track of `n` source frames yields
//! `round(n / rate)` frames, however it was split into packets. The decoder relies on that to
//! place the next track's mark and to keep gapless tracks gapless.

use crate::stretch::{BlockPlan, StretchError, Stretcher, BLOCK_FRAMES};
use crate::types::CHANNELS;
use crate::PlaybackError;

pub(crate) struct StretchStage {
    stretcher: Stretcher,
    plan: BlockPlan,
    active: bool,
    /// Source frames waiting for the next block (interleaved).
    input: Vec<f32>,
    /// One block of output.
    block: Vec<f32>,
    /// Output frames of lead-in still to drop.
    discard: usize,
    /// Source frames at the start of the input that are history before the start position.
    preroll: u64,
    /// Source frames pushed since `begin`, history included.
    pushed: u64,
    /// Output frames let through since `begin`.
    emitted: u64,
}

impl std::fmt::Debug for StretchStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StretchStage")
            .field("rate", &self.plan.rate())
            .field("active", &self.active)
            .field("pushed", &self.pushed)
            .field("emitted", &self.emitted)
            .finish_non_exhaustive()
    }
}

fn stretch_error(e: StretchError) -> PlaybackError {
    PlaybackError::Decode(format!("time stretch: {e}"))
}

impl StretchStage {
    /// A stage for tracks of `sample_rate` Hz (stereo). Allocates about a megabyte; reuse it for
    /// tracks of the same rate.
    ///
    /// # Errors
    /// The stretcher cannot be built for this rate (outside 8 kHz to 384 kHz) or out of memory.
    pub fn new(sample_rate: u32) -> Result<Self, PlaybackError> {
        let stretcher = Stretcher::new(CHANNELS, sample_rate).map_err(stretch_error)?;
        Ok(Self {
            stretcher,
            plan: BlockPlan::new(1.0).map_err(stretch_error)?,
            active: false,
            input: Vec::with_capacity(2 * BLOCK_FRAMES * CHANNELS),
            block: vec![0.0; BLOCK_FRAMES * CHANNELS],
            discard: 0,
            preroll: 0,
            pushed: 0,
            emitted: 0,
        })
    }

    /// How many source frames before the start position the reader should be positioned at (and
    /// pass to [`begin`](Self::begin) as `preroll`): the stretcher's analysis block, so that the
    /// first frame that is kept is computed from a full window of real audio.
    pub fn preroll_frames(&self) -> usize {
        2 * self.stretcher.input_latency_frames()
    }

    /// The file rate this stage was built for.
    pub fn sample_rate(&self) -> u32 {
        self.stretcher.sample_rate()
    }

    /// Whether a track is being stretched (between [`begin`](Self::begin) and
    /// [`finish`](Self::finish) / [`stop`](Self::stop)).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Start a track at `rate`: forgets everything before. The first `preroll` source frames
    /// that will be pushed are history; the audio the stream starts with is the frame after
    /// them.
    ///
    /// # Errors
    /// `rate` is not a positive finite number.
    pub fn begin(&mut self, rate: f64, preroll: usize) -> Result<(), PlaybackError> {
        self.active = false;
        let plan = BlockPlan::new(rate).map_err(stretch_error)?;
        // Frame `n` of the output is the source at `(n - lag) * rate` from where the feed began;
        // the first frame kept is the one at `preroll`.
        self.discard = self.stretcher.discard_frames(rate).map_err(stretch_error)?
            + (preroll as f64 / rate).round() as usize;
        self.preroll = preroll as u64;
        self.plan = plan;
        self.stretcher.reset();
        self.input.clear();
        self.pushed = 0;
        self.emitted = 0;
        self.active = true;
        Ok(())
    }

    /// Abandon the track (the stream restarted): its tail is not wanted.
    pub fn stop(&mut self) {
        self.active = false;
        self.input.clear();
    }

    /// Feed `frames` (interleaved source audio at the file rate) and append whatever output is
    /// ready to `out`. Output comes in blocks of [`BLOCK_FRAMES`]; less than a block of input
    /// stays inside until more arrives.
    ///
    /// # Errors
    /// The stretcher failed internally (a bug, not an input condition).
    pub fn push(&mut self, frames: &[f32], out: &mut Vec<f32>) -> Result<(), PlaybackError> {
        debug_assert!(self.active, "push before begin");
        debug_assert_eq!(frames.len() % CHANNELS, 0, "whole frames only");
        self.pushed += (frames.len() / CHANNELS) as u64;
        self.input.extend_from_slice(frames);
        loop {
            let need = self.plan.peek_input_frames(BLOCK_FRAMES);
            if self.input.len() < need * CHANNELS {
                return Ok(());
            }
            self.run_block(out)?;
        }
    }

    /// The track is over: run the tail out with silence and append it, so that afterwards exactly
    /// `round(pushed / rate)` frames were emitted in total. The stage is idle again.
    ///
    /// # Errors
    /// As [`push`](Self::push).
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), PlaybackError> {
        if !self.active {
            return Ok(());
        }
        let total =
            (self.pushed.saturating_sub(self.preroll) as f64 / self.plan.rate()).round() as u64;
        while self.emitted < total {
            let need = self.plan.peek_input_frames(BLOCK_FRAMES) * CHANNELS;
            if self.input.len() < need {
                self.input.resize(need, 0.0);
            }
            let before = out.len();
            self.run_block(out)?;
            // The last block may run past the end of the track: cut it there.
            let excess = self.emitted.saturating_sub(total);
            if excess > 0 {
                let cut = (excess as usize * CHANNELS).min(out.len() - before);
                out.truncate(out.len() - cut);
                self.emitted = total;
            }
        }
        self.stop();
        Ok(())
    }

    /// One block: take its input from the front of `input`, drop the lead-in, append the rest.
    fn run_block(&mut self, out: &mut Vec<f32>) -> Result<(), PlaybackError> {
        let take = self.plan.next_input_frames(BLOCK_FRAMES);
        self.stretcher
            .process(&self.input[..take * CHANNELS], &mut self.block)
            .map_err(stretch_error)?;
        self.input.drain(..take * CHANNELS);
        let skip = self.discard.min(BLOCK_FRAMES);
        self.discard -= skip;
        out.extend_from_slice(&self.block[skip * CHANNELS..]);
        self.emitted += (BLOCK_FRAMES - skip) as u64;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // test code: a failed expectation is the failure message
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    fn tone(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|n| {
                let t = n as f64 / f64::from(SR);
                [
                    (0.5 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32,
                    (0.25 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32,
                ]
            })
            .collect()
    }

    fn run(rate: f64, input: &[f32], step: usize) -> Vec<f32> {
        let mut stage = StretchStage::new(SR).expect("stage");
        stage.begin(rate, 0).expect("begin");
        let mut out = Vec::new();
        for chunk in input.chunks(step * CHANNELS) {
            stage.push(chunk, &mut out).expect("push");
        }
        stage.finish(&mut out).expect("finish");
        out
    }

    #[test]
    fn a_track_comes_out_round_n_over_rate_frames_long() {
        for rate in [0.75, 1.1, 1.25, 1.5] {
            for frames in [0usize, 1, 700, 5_000, 44_100, 100_003] {
                let out = run(rate, &tone(frames), 1_152);
                let expected = (frames as f64 / rate).round() as usize;
                assert_eq!(
                    out.len() / CHANNELS,
                    expected,
                    "rate {rate}, {frames} source frames"
                );
            }
        }
    }

    #[test]
    fn the_output_does_not_depend_on_how_the_input_is_split() {
        let input = tone(30_000);
        let whole = run(1.25, &input, 30_000);
        assert!(whole.iter().any(|&s| s != 0.0));
        for step in [1, 147, 1_152, 4_096] {
            assert_eq!(run(1.25, &input, step), whole, "pushes of {step} frames");
        }
    }

    #[test]
    fn a_stage_is_reusable_and_a_new_track_starts_clean() {
        let a = tone(20_000);
        let mut stage = StretchStage::new(SR).expect("stage");
        let mut first = Vec::new();
        stage.begin(1.25, 0).expect("begin");
        stage.push(&a, &mut first).expect("push");
        stage.finish(&mut first).expect("finish");
        assert!(!stage.is_active());

        stage.begin(1.25, 0).expect("begin again");
        let mut second = Vec::new();
        stage.push(&a, &mut second).expect("push");
        stage.finish(&mut second).expect("finish");
        assert_eq!(first, second, "history survived into the next track");
    }

    #[test]
    fn a_stopped_track_leaves_nothing_behind() {
        let a = tone(20_000);
        let mut stage = StretchStage::new(SR).expect("stage");
        let mut ignored = Vec::new();
        stage.begin(0.75, 0).expect("begin");
        stage.push(&a[..8_000], &mut ignored).expect("push");
        stage.stop();
        assert!(!stage.is_active());
        let mut out = Vec::new();
        stage.begin(0.75, 0).expect("begin");
        stage.push(&a, &mut out).expect("push");
        stage.finish(&mut out).expect("finish");
        assert_eq!(out, run(0.75, &a, 20_000));
    }

    #[test]
    fn begin_refuses_a_rate_that_makes_no_sense() {
        let mut stage = StretchStage::new(SR).expect("stage");
        for rate in [0.0, -1.0, f64::NAN] {
            assert!(stage.begin(rate, 0).is_err(), "rate {rate}");
        }
        assert!(!stage.is_active());
    }

    #[test]
    fn a_rate_the_stretcher_cannot_be_built_for_is_an_error() {
        assert!(StretchStage::new(1_000).is_err());
    }
}
