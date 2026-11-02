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
    /// This renderer has taken audio from its ring since it was opened or last flushed. Until it
    /// has, an empty ring is "waiting for the first audio" (start, seek, device switch), not an
    /// underrun. Per renderer, so a new stream never inherits the old stream's state.
    seen_data: bool,
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
            seen_data: false,
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
            self.seen_data = false;
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
        self.seen_data |= popped_len > 0;
        if popped_len < len && !eos && self.seen_data {
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering::{Relaxed, Release};

    use rtrb::RingBuffer;

    use super::*;

    fn rig() -> (Arc<Shared>, rtrb::Producer<f32>, Renderer) {
        let sh = Shared::new(1.0);
        sh.eos.store(false, Release);
        let (prod, cons) = RingBuffer::new(4_800);
        let renderer = Renderer::new(cons, Arc::clone(&sh));
        (sh, prod, renderer)
    }

    #[test]
    fn waiting_for_the_first_audio_is_not_an_underrun() {
        let (sh, mut prod, mut r) = rig();
        let mut out = [0.0f32; 480];
        for _ in 0..50 {
            assert_eq!(r.render(&mut out, 0), 0);
        }
        assert_eq!(sh.underruns.load(Relaxed), 0);

        // Once audio has flowed, running dry is.
        let _ = prod.push_partial_slice(&[0.25; 240]);
        assert_eq!(r.render(&mut out, 0), 120);
        assert_eq!(
            sh.underruns.load(Relaxed),
            1,
            "partial block: starved mid-stream"
        );
        r.render(&mut out, 0);
        assert_eq!(sh.underruns.load(Relaxed), 2);
    }

    #[test]
    fn a_flush_returns_the_renderer_to_waiting() {
        let (sh, mut prod, mut r) = rig();
        let mut out = [0.0f32; 480];
        let _ = prod.push_partial_slice(&[0.25; 480]);
        r.render(&mut out, 0);
        sh.flush_req.fetch_add(1, Release);
        r.render(&mut out, 0); // drains and acknowledges
        for _ in 0..20 {
            r.render(&mut out, 0);
        }
        assert_eq!(
            sh.underruns.load(Relaxed),
            0,
            "refilling after a seek is not starvation"
        );
    }

    #[test]
    fn a_new_renderer_does_not_inherit_the_old_ones_state() {
        let (sh, mut prod, mut old) = rig();
        let mut out = [0.0f32; 480];
        let _ = prod.push_partial_slice(&[0.25; 960]);
        old.render(&mut out, 0);
        old.render(&mut out, 0);
        drop(old);
        // A device switch builds a new renderer on the same shared state; its ring is empty until
        // the decoder has switched over, whatever the old stream had seen.
        let (_prod, cons) = RingBuffer::<f32>::new(4_800);
        let mut fresh = Renderer::new(cons, Arc::clone(&sh));
        for _ in 0..50 {
            fresh.render(&mut out, 0);
        }
        assert_eq!(sh.underruns.load(Relaxed), 0);
    }
}
