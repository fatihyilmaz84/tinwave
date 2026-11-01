//! The real-time half of the engine. [`Renderer::render`] runs inside the OS audio callback.

use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::Arc;

use rtrb::Consumer;

use crate::shared::Shared;
use crate::types::CHANNELS;

/// Fills output buffers from the decoder ring. Handed to the [`AudioOutput`](crate::AudioOutput)
/// backend, which calls [`render`](Self::render) from its audio callback.
///
/// # Real-time contract
/// `render` never allocates, never locks, never performs IO and never waits for another thread:
/// it works on atomics and the lock-free ring only. This is enforced by `tests/rt_alloc.rs`
/// (counting global allocator) and `tests/blocking_reader.rs` (a stalled decoder does not
/// stall the callback). Do not add anything to this file that could break it.
#[derive(Debug)]
pub struct Renderer {
    cons: Consumer<f32>,
    sh: Arc<Shared>,
    /// Volume gain at the end of the previous block.
    gain: f32,
    /// 1.0 playing, 0.0 paused; pause and resume are ramped across one block to avoid clicks.
    pause_level: f32,
}

impl Renderer {
    pub(crate) fn new(cons: Consumer<f32>, sh: Arc<Shared>) -> Self {
        let gain = sh.volume();
        let pause_level = if sh.paused.load(Relaxed) { 0.0 } else { 1.0 };
        Self {
            cons,
            sh,
            gain,
            pause_level,
        }
    }

    /// Frames of decoded audio waiting in the ring.
    #[must_use]
    pub fn buffered_frames(&self) -> usize {
        self.cons.slots() / CHANNELS
    }

    /// Fill `out` (interleaved stereo `f32`) and return how many frames carry real audio; the rest
    /// is silence. `latency_frames` is the delay between this write and the speaker, used only
    /// for position reporting.
    pub fn render(&mut self, out: &mut [f32], latency_frames: u64) -> usize {
        let sh = &*self.sh;
        sh.latency_frames.store(latency_frames, Relaxed);

        // A stray odd sample cannot hold a frame.
        let len = out.len() & !1;
        let (out, tail) = out.split_at_mut(len);
        tail.fill(0.0);

        // Seek / device handshake: while the decoder is parked, discard everything in the ring.
        let req = sh.flush_req.load(Acquire);
        if req != sh.flush_ack.load(Relaxed) {
            let queued = self.cons.slots();
            if let Ok(chunk) = self.cons.read_chunk(queued) {
                chunk.commit_all();
            }
            sh.flush_ack.store(req, Release);
            out.fill(0.0);
            return 0;
        }

        let want_pause = sh.paused.load(Relaxed);
        let pause_target = if want_pause { 0.0 } else { 1.0 };
        if want_pause && self.pause_level == 0.0 {
            out.fill(0.0);
            return 0;
        }

        // Read the end-of-stream flag BEFORE the ring: "eos and nothing left" is then conclusive.
        let eos = sh.eos.load(Acquire);
        let (popped, rest) = self.cons.pop_partial_slice(out);
        let popped_len = popped.len();
        rest.fill(0.0);
        if popped_len < len && !eos && sh.primed.load(Acquire) {
            sh.underruns.fetch_add(1, Relaxed);
        }

        let frames = popped_len / CHANNELS;
        if frames == 0 {
            self.gain = sh.volume();
            self.pause_level = pause_target;
            return 0;
        }

        // Volume and pause both ramp linearly across the block: no zipper noise, no clicks.
        let volume_target = sh.volume();
        let start = self.gain * self.pause_level;
        let end = volume_target * pause_target;
        if start == 1.0 && end == 1.0 {
            // Unity gain: leave the samples bit-exact.
        } else if (end - start).abs() < f32::EPSILON {
            for sample in &mut out[..popped_len] {
                *sample *= start;
            }
        } else {
            let step = (end - start) / frames as f32;
            let mut gain = start;
            for frame in out[..popped_len].chunks_exact_mut(CHANNELS) {
                gain += step;
                frame[0] *= gain;
                frame[1] *= gain;
            }
        }
        self.gain = volume_target;
        self.pause_level = pause_target;

        sh.consumed.fetch_add(frames as u64, Release);
        frames
    }
}
