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
///
/// # Playback rate
/// Time-stretch happens on the decoder thread; the callback only does bookkeeping and fades. Every
/// block that carries audio adds its frames to `consumed` (wall frames) and its frames times the
/// stream's rate to `src_micro`, and publishes that rate as `eff_rate`. A rate change is a flush
/// the decoder asks for: the callback fades out at most 5 ms of what is queued (counted at the old
/// rate), drops the rest, and fades the new stream in over 5 ms.
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
    /// Frames of the fade-in that a rate-change flush started, still to apply to the next audio.
    fade_in_left: usize,
    /// A rate-change flush is fading out the queued audio: `(frames left, frames in all)`. It
    /// spans as many callbacks as it takes, so a device with a buffer shorter than the fade
    /// still gets the whole ramp.
    fade_out: Option<(usize, usize)>,
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
            fade_in_left: 0,
            fade_out: None,
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

        // Seek / device / rate handshake: while the decoder is parked, discard everything in the
        // ring.
        let req = sh.flush_req.load(Acquire);
        if req != sh.flush_ack.load(Relaxed) {
            return self.flush(out, req);
        }

        let want_pause = sh.paused.load(Relaxed);
        let pause_target = if want_pause { 0.0 } else { 1.0 };
        if want_pause && self.pause_level == 0.0 {
            out.fill(0.0);
            return 0;
        }

        // Read the end-of-stream flag BEFORE the ring: "eos and nothing left" is then conclusive.
        let eos = sh.eos.load(Acquire);
        // `buffering` is set before the decoder revives an ended queue, so seeing the revived
        // stream (`eos` false) implies seeing this too.
        let buffering = sh.buffering.load(Acquire);
        let (popped, rest) = self.cons.pop_partial_slice(out);
        let popped_len = popped.len();
        rest.fill(0.0);
        self.seen_data |= popped_len > 0;
        if popped_len < len && !eos && !buffering && self.seen_data {
            sh.underruns.fetch_add(1, Relaxed);
        }

        let frames = popped_len / CHANNELS;
        if frames == 0 {
            // The device buffer is emptying: the engine waits for it before it replaces a stream.
            sh.idle_frames.fetch_add((len / CHANNELS) as u64, Relaxed);
            self.gain = sh.volume();
            self.pause_level = pause_target;
            return 0;
        }
        sh.idle_frames.store(0, Relaxed);

        self.fade_in(&mut out[..popped_len]);
        self.shape(&mut out[..popped_len], pause_target);
        self.account(frames);
        frames
    }

    /// The flush handshake. A plain flush (seek, load, device change) drops everything and
    /// returns silence. A rate-change flush first plays up to 5 ms of the queued audio, fading it
    /// to zero (over as many callbacks as that takes, so the ramp is whole whatever the buffer
    /// size), so that the old stream ends without a click, and arms the fade-in of the next one.
    /// The decoder is acknowledged when the fade is done.
    fn flush(&mut self, out: &mut [f32], req: u64) -> usize {
        let fade = self.sh.flush_fade.load(Acquire);
        let fade_frames = self.sh.fade_frames.load(Relaxed) as usize;
        // A callback with no room (the tests' empty acknowledgement) cannot play a fade: cut.
        if fade
            && self.fade_out.is_none()
            && !out.is_empty()
            && self.pause_level > 0.0
            && !self.sh.paused.load(Relaxed)
        {
            let total = fade_frames.min(self.cons.slots() / CHANNELS);
            self.fade_out = Some((total, total));
        }
        let mut played = 0;
        if let Some((left, total)) = self.fade_out {
            let wanted = left.min(out.len() / CHANNELS);
            if wanted > 0 {
                let (popped, _) = self.cons.pop_partial_slice(&mut out[..wanted * CHANNELS]);
                played = popped.len() / CHANNELS;
            }
            let pause_target = if self.sh.paused.load(Relaxed) {
                0.0
            } else {
                1.0
            };
            let block = &mut out[..played * CHANNELS];
            self.shape(block, pause_target);
            // Frame `j` of the fade has gain `1 - (j + 1) / total`: the last one is silent.
            for (i, frame) in block.chunks_exact_mut(CHANNELS).enumerate() {
                let g = (left - i - 1) as f32 / total as f32;
                frame[0] *= g;
                frame[1] *= g;
            }
            if played > 0 {
                self.account(played);
            }
            // A ring that ran short ends the fade where the audio ends.
            let left = if played < wanted { 0 } else { left - played };
            self.fade_out = (left > 0).then_some((left, total));
            if left > 0 {
                out[played * CHANNELS..].fill(0.0);
                return played;
            }
        }
        out[played * CHANNELS..].fill(0.0);
        let queued = self.cons.slots();
        if let Ok(chunk) = self.cons.read_chunk(queued) {
            chunk.commit_all();
        }
        self.fade_in_left = if fade { fade_frames } else { 0 };
        self.seen_data = false;
        // Last: the decoder reads `consumed` once it sees the acknowledgement.
        self.sh.flush_ack.store(req, Release);
        played
    }

    /// Ramp up from silence over the frames left of the fade-in a rate change started.
    fn fade_in(&mut self, block: &mut [f32]) {
        if self.fade_in_left == 0 {
            return;
        }
        let total = (self.sh.fade_frames.load(Relaxed) as usize).max(1);
        let frames = (block.len() / CHANNELS).min(self.fade_in_left);
        for (i, frame) in block.chunks_exact_mut(CHANNELS).take(frames).enumerate() {
            let g = (total.saturating_sub(self.fade_in_left) + i) as f32 / total as f32;
            frame[0] *= g;
            frame[1] *= g;
        }
        self.fade_in_left -= frames;
    }

    /// Volume and pause both ramp linearly across the block: no zipper noise, no clicks.
    fn shape(&mut self, block: &mut [f32], pause_target: f32) {
        let frames = block.len() / CHANNELS;
        let volume_target = self.sh.volume();
        let start = self.gain * self.pause_level;
        let end = volume_target * pause_target;
        if start == 1.0 && end == 1.0 {
            // Unity gain: leave the samples bit-exact.
        } else if (end - start).abs() < f32::EPSILON {
            for sample in block.iter_mut() {
                *sample *= start;
            }
        } else {
            let step = (end - start) / frames as f32;
            let mut gain = start;
            for frame in block.chunks_exact_mut(CHANNELS) {
                gain += step;
                frame[0] *= gain;
                frame[1] *= gain;
            }
        }
        self.gain = volume_target;
        self.pause_level = pause_target;
    }

    /// Book `frames` of audio that went to the device: wall frames, source micro-frames at the
    /// rate of the stream they came from, and that rate as the effective one.
    fn account(&self, frames: usize) {
        let sh = &*self.sh;
        // Read after the pop that returned these frames: the decoder stores the rate before it
        // pushes the first frame at that rate.
        let rate = sh.stream_rate();
        sh.src_micro
            .fetch_add((frames as f64 * rate * 1e6) as u64, Relaxed);
        sh.eff_rate.store(rate.to_bits(), Relaxed);
        sh.consumed.fetch_add(frames as u64, Release);
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
    fn a_revived_queue_is_buffering_until_its_first_push() {
        let (sh, mut prod, mut r) = rig();
        let mut out = [0.0f32; 480];
        let _ = prod.push_partial_slice(&[0.25; 480]);
        r.render(&mut out, 0);
        // `Append` after the end: the decoder announces buffering, then revives the stream.
        sh.buffering.store(true, Release);
        sh.eos.store(false, Release);
        for _ in 0..20 {
            r.render(&mut out, 0);
        }
        assert_eq!(
            sh.underruns.load(Relaxed),
            0,
            "opening the file is buffering"
        );
        // First push: audio is in the ring, buffering over; running dry again counts.
        let _ = prod.push_partial_slice(&[0.25; 240]);
        sh.buffering.store(false, Release);
        r.render(&mut out, 0);
        assert_eq!(sh.underruns.load(Relaxed), 1);
    }

    #[test]
    fn silence_after_the_last_audio_is_counted_until_audio_comes_back() {
        let (sh, mut prod, mut r) = rig();
        let mut out = [0.0f32; 960]; // 480 frames
        let _ = prod.push_partial_slice(&[0.25; 240]);
        // A block that still carries audio is not idle, even if it ends in silence.
        assert_eq!(r.render(&mut out, 0), 120);
        assert_eq!(sh.idle_frames.load(Relaxed), 0);
        // From here the device buffer is emptying: every silent block adds its frames.
        r.render(&mut out, 0);
        assert_eq!(sh.idle_frames.load(Relaxed), 480);
        r.render(&mut out, 0);
        assert_eq!(sh.idle_frames.load(Relaxed), 960);
        // Audio resets the count.
        let _ = prod.push_partial_slice(&[0.25; 960]);
        assert_eq!(r.render(&mut out, 0), 480);
        assert_eq!(sh.idle_frames.load(Relaxed), 0);
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

    /// A ring of constant audio (`1.0` on every sample) at stream rate `rate`, fade of 100 frames.
    fn rate_rig(rate: f64, frames: usize) -> (Arc<Shared>, Renderer) {
        let (sh, mut prod, r) = rig();
        sh.fade_frames.store(100, Relaxed);
        sh.stream_rate.store(rate.to_bits(), Release);
        let _ = prod.push_partial_slice(&vec![1.0; frames * CHANNELS]);
        (sh, r)
    }

    #[test]
    fn every_block_books_wall_frames_source_micro_frames_and_the_effective_rate() {
        let (sh, mut r) = rate_rig(1.25, 480);
        let mut out = [0.0f32; 480];
        assert_eq!(r.render(&mut out, 0), 240);
        assert_eq!(sh.consumed.load(Relaxed), 240);
        assert_eq!(sh.src_micro.load(Relaxed), 300_000_000);
        assert_eq!(f64::from_bits(sh.eff_rate.load(Relaxed)), 1.25);
        assert_eq!(r.render(&mut out, 0), 240);
        assert_eq!(sh.consumed.load(Relaxed), 480);
        // A block with no audio books nothing and leaves the last effective rate alone.
        assert_eq!(r.render(&mut out, 0), 0);
        assert_eq!(sh.consumed.load(Relaxed), 480);
        assert_eq!(sh.src_micro.load(Relaxed), 600_000_000);
        assert_eq!(f64::from_bits(sh.eff_rate.load(Relaxed)), 1.25);
    }

    #[test]
    fn bypass_books_exactly_one_source_frame_per_wall_frame() {
        let (sh, mut r) = rate_rig(1.0, 2_000);
        let mut out = [0.0f32; 2 * 441];
        for _ in 0..4 {
            r.render(&mut out, 0);
        }
        assert_eq!(sh.consumed.load(Relaxed), 4 * 441);
        assert_eq!(sh.src_micro.load(Relaxed), 4 * 441 * 1_000_000);
    }

    #[test]
    fn a_rate_flush_plays_a_fade_out_at_the_old_rate_then_drops_the_rest() {
        let (sh, mut r) = rate_rig(1.5, 1_000);
        let mut out = [0.0f32; 2 * 480];
        sh.flush_fade.store(true, Release);
        sh.flush_req.fetch_add(1, Release);
        // 480 frames asked, 100 fade frames: exactly those are played, ending at zero.
        assert_eq!(r.render(&mut out, 0), 100);
        assert_eq!(sh.flush_ack.load(Relaxed), 1);
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        assert!(
            left[..100].windows(2).all(|w| w[1] < w[0]),
            "monotonic fade"
        );
        assert!(
            left[0] > 0.98 && left[99] == 0.0,
            "{} .. {}",
            left[0],
            left[99]
        );
        assert!(left[100..].iter().all(|&s| s == 0.0));
        // Booked as played, at the old rate; everything else was dropped.
        assert_eq!(sh.consumed.load(Relaxed), 100);
        assert_eq!(sh.src_micro.load(Relaxed), 150_000_000);
        assert_eq!(r.buffered_frames(), 0);
    }

    #[test]
    fn a_rate_flush_in_a_small_buffer_spans_callbacks_and_still_ends_at_zero() {
        let (sh, mut r) = rate_rig(1.0, 1_000);
        let mut out = [0.0f32; 2 * 40];
        sh.flush_fade.store(true, Release);
        sh.flush_req.fetch_add(1, Release);
        // 100 frames of fade in callbacks of 40: 40, 40, 20. Acknowledged only when it is done.
        let mut ramp = Vec::new();
        for (callback, expected) in [(0, 40), (1, 40), (2, 20)] {
            assert_eq!(r.render(&mut out, 0), expected, "callback {callback}");
            ramp.extend(out.chunks_exact(2).take(expected).map(|f| f[0]));
            let acked = sh.flush_ack.load(Relaxed);
            assert_eq!(acked, u64::from(callback == 2), "callback {callback}");
        }
        assert!(ramp.windows(2).all(|w| w[1] < w[0]), "one monotonic ramp");
        assert!(ramp[0] > 0.98);
        assert_eq!(ramp[99], 0.0, "ends at zero");
        assert_eq!(sh.consumed.load(Relaxed), 100);
        assert_eq!(r.buffered_frames(), 0, "the rest is dropped");
    }

    #[test]
    fn a_rate_flush_with_little_queued_fades_what_there_is() {
        let (sh, mut r) = rate_rig(1.0, 30);
        let mut out = [0.0f32; 2 * 480];
        sh.flush_fade.store(true, Release);
        sh.flush_req.fetch_add(1, Release);
        assert_eq!(r.render(&mut out, 0), 30);
        assert_eq!(out[2 * 29], 0.0);
    }

    #[test]
    fn the_new_stream_after_a_rate_flush_fades_in_over_the_fade_length() {
        let (sh, mut prod, mut r) = rig();
        sh.fade_frames.store(100, Relaxed);
        let mut out = [0.0f32; 2 * 480];
        sh.flush_fade.store(true, Release);
        sh.flush_req.fetch_add(1, Release);
        assert_eq!(r.render(&mut out, 0), 0, "nothing queued: nothing to fade");
        let _ = prod.push_partial_slice(&vec![1.0f32; 2 * 480]);
        assert_eq!(r.render(&mut out, 0), 480);
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        assert_eq!(left[0], 0.0, "starts from silence");
        assert!(
            left[..100].windows(2).all(|w| w[1] > w[0]),
            "monotonic fade-in"
        );
        assert!((left[50] - 0.5).abs() < 0.01);
        assert!(
            left[100..].iter().all(|&s| s == 1.0),
            "unity again, bit-exact"
        );
        // Once only: the next block is untouched.
        let _ = prod.push_partial_slice(&vec![1.0f32; 2 * 480]);
        r.render(&mut out, 0);
        assert!(out.iter().all(|&s| s == 1.0));
    }

    #[test]
    fn a_plain_flush_neither_fades_nor_books_anything() {
        let (sh, mut r) = rate_rig(1.0, 1_000);
        let mut out = [0.5f32; 2 * 480];
        sh.flush_fade.store(false, Release);
        sh.flush_req.fetch_add(1, Release);
        assert_eq!(r.render(&mut out, 0), 0);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(sh.consumed.load(Relaxed), 0);
        assert_eq!(sh.src_micro.load(Relaxed), 0);
        assert_eq!(r.buffered_frames(), 0);
    }

    #[test]
    fn a_paused_renderer_flushes_without_fading() {
        let (sh, mut r) = rate_rig(1.0, 1_000);
        let mut out = [0.0f32; 2 * 480];
        sh.paused.store(true, Relaxed);
        r.render(&mut out, 0); // ramps down to paused
        let before = sh.consumed.load(Relaxed);
        sh.flush_fade.store(true, Release);
        sh.flush_req.fetch_add(1, Release);
        assert_eq!(
            r.render(&mut out, 0),
            0,
            "nothing is audible, so nothing is faded"
        );
        assert_eq!(sh.consumed.load(Relaxed), before);
        assert_eq!(r.buffered_frames(), 0);
    }
}
