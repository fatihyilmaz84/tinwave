//! State shared by the control thread, the decoder thread and the render callback.
//!
//! The render callback only touches the atomics, never `marks`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::Track;

/// Why a mark was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkKind {
    /// First mark after a load, seek or device change: a command (already announced) caused it.
    Initial,
    /// The decoder moved on to the next queue item by itself.
    Continuation,
    /// The decoder restarted the stream at a new playback rate, at the position that was playing:
    /// the same item as the mark before it (nothing to announce), unless the old mark of the next
    /// item is still ahead of what the control side has heard, in which case that mark announces it.
    Rate,
}

/// "Output frame `start_frame` is frame `offset` of `item`, and from there the source advances
/// `rate` frames per output frame."
///
/// `start_frame` counts frames the renderer has consumed since the engine started; it never
/// resets, so positions stay comparable across seeks and device changes. Those frames are
/// counted at the rate of the stream that played them (`out_rate`), which changes when an
/// exclusive-mode stream is reopened at a file's own rate (T-0057). `offset` and every position
/// derived from it are in **source timeline frames**: the track's own timeline at the engine's
/// timeline rate (`timeline_rate`, the one rate every position, seek target and start frame
/// the control side sees is counted in), whatever the playback rate and the stream rate (a
/// timeline frame of the source is `1 / timeline_rate` seconds of the track).
#[derive(Debug, Clone)]
pub(crate) struct Mark {
    pub seq: u64,
    pub start_frame: u64,
    pub item: Arc<Track>,
    pub offset: u64,
    pub kind: MarkKind,
    /// The item before this one failed mid-decode instead of finishing.
    pub prev_failed: bool,
    /// Source frames per output frame from `start_frame` on: 1.0 is bypass (the exact path).
    pub rate: f64,
    /// Sample rate of the output stream the frames from `start_frame` on are played at.
    pub out_rate: u32,
    /// The rate `offset` and the positions derived from it are counted at.
    pub timeline_rate: u32,
}

impl Mark {
    /// The source frame heard at output frame `frame` (at or after `start_frame`).
    pub fn source_frame_at(&self, frame: u64) -> u64 {
        let elapsed = frame.saturating_sub(self.start_frame);
        if self.rate == 1.0 {
            // Bypass stays integer arithmetic: nothing in the float path can touch it. When the
            // stream runs at the timeline rate (every shared-mode stream) it is the sum below.
            return self.offset + self.timeline_frames(elapsed);
        }
        // `as` saturates; an elapsed count that large cannot be a real position.
        let source = elapsed as f64 * self.rate;
        let source = if self.out_rate == self.timeline_rate {
            source
        } else {
            source * f64::from(self.timeline_rate) / f64::from(self.out_rate.max(1))
        };
        self.offset + source.round() as u64
    }

    /// `frames` output frames at the stream rate, as timeline frames (rounded to nearest).
    fn timeline_frames(&self, frames: u64) -> u64 {
        if self.out_rate == self.timeline_rate {
            frames
        } else {
            crate::resample::scale_frames(frames, self.out_rate, self.timeline_rate)
        }
    }
}

#[derive(Debug)]
pub(crate) struct Shared {
    /// Frames the renderer has taken from the ring. Monotonic, never reset. This is also the
    /// wall-clock side of the rate bookkeeping (`wall_frames`): frames of real audio played.
    pub consumed: AtomicU64,
    /// Frames between a callback write and the speaker, as reported by the output.
    pub latency_frames: AtomicU64,
    /// Target gain, `f32::to_bits`.
    pub volume: AtomicU32,
    pub paused: AtomicBool,
    /// Seek handshake: the decoder raises `flush_req`, the renderer drains the ring and copies it
    /// to `flush_ack`.
    pub flush_req: AtomicU64,
    pub flush_ack: AtomicU64,
    /// The flush that `flush_req` announces is a rate change at the audible position: the
    /// renderer fades out up to `fade_frames` of what is queued and fades the new stream in,
    /// instead of cutting. Written before `flush_req` is raised.
    pub flush_fade: AtomicBool,
    /// Length of a rate-change fade in frames (5 ms at the stream rate).
    pub fade_frames: AtomicU32,
    /// The playback rate the listener asked for (`f64::to_bits`). The decoder applies it, at most
    /// one flush per throttle interval.
    pub rate_target: AtomicU64,
    /// The playback rate of the audio in the ring (`f64::to_bits`): source frames per output
    /// frame. Constant between flushes, because a rate change is a flush: the decoder stores it
    /// while the ring is empty (after the renderer acknowledged), before it pushes the first frame
    /// that has this rate, so the renderer reads it after popping and always sees the rate of the
    /// frames it popped. (ADR 0006 decision 5 sketched a segment queue for rates that change
    /// inside a ring; with flush-and-restart there is never more than one rate in it.)
    pub stream_rate: AtomicU64,
    /// Source micro-frames (millionths of a frame) the renderer has played at `stream_rate`.
    /// Monotonic; `src_micro / 1e6 / consumed` is the time-weighted rate.
    pub src_micro: AtomicU64,
    /// Effective rate of the last block that carried audio (`f64::to_bits`).
    pub eff_rate: AtomicU64,
    /// Rate flushes the decoder has carried out (the stream restarted at the new rate).
    pub rate_flushes: AtomicU64,
    /// The decoder has pushed everything; `eos_total` holds the final stream frame count.
    pub eos: AtomicBool,
    pub eos_total: AtomicU64,
    /// The last item before the end of stream failed mid-decode.
    pub eos_failed: AtomicBool,
    /// The queue was revived by `Append` and the decoder has not pushed the first frame of the new
    /// track yet (it is still opening the file). The renderer's ring is empty and its `seen_data`
    /// is still true from the previous track, but waiting is buffering, not an underrun.
    /// Set by the decoder before it clears `eos`, cleared after its next push (or `finish_queue`).
    pub buffering: AtomicBool,
    pub underruns: AtomicU64,
    /// Highest control-command generation whose effect (first mark or end of stream) is published.
    pub applied_gen: AtomicU64,
    pub shutdown: AtomicBool,
    /// Frames of silence the renderer has produced since it last played audio (the audio ran out
    /// and the device buffer is emptying). A single writer: the renderer.
    pub idle_frames: AtomicU64,
    /// The output stream is exclusive, so the decoder asks for a stream at each file's own rate
    /// (T-0057). Written by the engine whenever it opens a stream.
    pub follow: AtomicBool,
    /// The decoder is parked, waiting for the engine to reopen the output stream at a new rate:
    /// `(id << 32) | rate`, rate 0 while it is not. See [`Shared::park`] for the other `reopen_*`
    /// fields, which are stored before this one.
    pub reopen: AtomicU64,
    /// Stream frames the old stream must have played before it is replaced.
    pub reopen_total: AtomicU64,
    /// The old stream must also empty its device buffer first: its tail is audio the listener has
    /// not heard yet (a track boundary), not audio a seek or load already threw away.
    pub reopen_drain: AtomicBool,
    /// Generation of the last restarting command (load, seek, stop, device switch) the decoder has
    /// taken from its queue, stored after that command cancelled any park. The engine reopens only
    /// when this is the generation of its latest command, and reads it before the request, so a
    /// command that is still queued can never meet a reopen of a stale park.
    pub reopen_gen: AtomicU64,
    marks: Mutex<VecDeque<Mark>>,
    refused: Mutex<Vec<u32>>,
}

impl Shared {
    pub fn new(volume_linear: f32) -> Arc<Self> {
        Arc::new(Self {
            consumed: AtomicU64::new(0),
            latency_frames: AtomicU64::new(0),
            volume: AtomicU32::new(volume_linear.to_bits()),
            paused: AtomicBool::new(false),
            flush_req: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            flush_fade: AtomicBool::new(false),
            // 5 ms at 48 kHz; the engine sets the real value for its stream rate.
            fade_frames: AtomicU32::new(240),
            rate_target: AtomicU64::new(1.0f64.to_bits()),
            stream_rate: AtomicU64::new(1.0f64.to_bits()),
            src_micro: AtomicU64::new(0),
            eff_rate: AtomicU64::new(1.0f64.to_bits()),
            rate_flushes: AtomicU64::new(0),
            eos: AtomicBool::new(true),
            eos_total: AtomicU64::new(0),
            eos_failed: AtomicBool::new(false),
            buffering: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            applied_gen: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            idle_frames: AtomicU64::new(0),
            follow: AtomicBool::new(false),
            reopen: AtomicU64::new(0),
            reopen_total: AtomicU64::new(0),
            reopen_drain: AtomicBool::new(false),
            reopen_gen: AtomicU64::new(0),
            marks: Mutex::new(VecDeque::new()),
            refused: Mutex::new(Vec::new()),
        })
    }

    /// The decoder asks for a stream at `rate`: everything the engine needs to decide when to
    /// replace the stream is stored first, then the request itself, so an engine that sees the
    /// request sees the rest (it reads them in the opposite order).
    pub fn park(&self, id: u32, rate: u32, total: u64, drain: bool) {
        self.reopen_total.store(total, Ordering::Release);
        self.reopen_drain.store(drain, Ordering::Release);
        self.reopen
            .store(pack_reopen(id, rate), Ordering::Release);
    }

    /// The decoder is no longer waiting for a new stream.
    pub fn unpark(&self, id: u32) {
        self.reopen.store(pack_reopen(id, 0), Ordering::Release);
    }

    /// The pending reopen request as `(id, rate)`; rate 0 means none.
    pub fn reopen_request(&self) -> (u32, u32) {
        unpack_reopen(self.reopen.load(Ordering::Acquire))
    }

    /// Rates the output stream refused in exclusive mode: tracks of those rates are resampled.
    pub fn refused(&self) -> MutexGuard<'_, Vec<u32>> {
        self.refused.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    /// The rate the listener asked for.
    pub fn rate_target(&self) -> f64 {
        f64::from_bits(self.rate_target.load(Ordering::Acquire))
    }

    /// The rate of the audio in the ring.
    pub fn stream_rate(&self) -> f64 {
        f64::from_bits(self.stream_rate.load(Ordering::Acquire))
    }

    /// Lock the mark list. Only the decoder and control threads call this, never the renderer.
    pub fn marks(&self) -> MutexGuard<'_, VecDeque<Mark>> {
        self.marks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn pack_reopen(id: u32, rate: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(rate)
}

fn unpack_reopen(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::QueueItemId;

    #[test]
    fn a_reopen_request_round_trips_and_clears_to_no_rate() {
        let sh = Shared::new(1.0);
        assert_eq!(sh.reopen_request(), (0, 0));
        sh.park(7, 96_000, 1234, true);
        assert_eq!(sh.reopen_request(), (7, 96_000));
        assert_eq!(sh.reopen_total.load(Ordering::Acquire), 1234);
        sh.unpark(7);
        assert_eq!(sh.reopen_request(), (7, 0));
        sh.park(u32::MAX, u32::MAX, 0, false);
        assert_eq!(sh.reopen_request(), (u32::MAX, u32::MAX));
    }

    #[test]
    fn frames_of_a_stream_at_another_rate_count_in_timeline_frames() {
        // A 96 kHz stream on a 48 kHz timeline: two output frames are one timeline frame.
        let mut m = mark(1_000, 500, 1.0);
        m.out_rate = 96_000;
        m.timeline_rate = 48_000;
        assert_eq!(m.source_frame_at(1_000), 500);
        assert_eq!(m.source_frame_at(1_000 + 96_000), 500 + 48_000);
        // 44.1 kHz stream: 441 output frames are exactly 480 timeline frames.
        let mut slow = mark(0, 0, 1.0);
        slow.out_rate = 44_100;
        slow.timeline_rate = 48_000;
        assert_eq!(slow.source_frame_at(441), 480);
        // With a playback rate the same conversion applies after the rate.
        let mut fast = mark(0, 0, 1.5);
        fast.out_rate = 96_000;
        fast.timeline_rate = 48_000;
        assert_eq!(fast.source_frame_at(96_000), 72_000);
    }

    fn mark(start_frame: u64, offset: u64, rate: f64) -> Mark {
        Mark {
            seq: 0,
            start_frame,
            item: Arc::new(Track {
                id: QueueItemId(1),
                path: PathBuf::from("unused.wav"),
                gapless: None,
            }),
            offset,
            kind: MarkKind::Rate,
            prev_failed: false,
            rate,
            out_rate: 48_000,
            timeline_rate: 48_000,
        }
    }

    #[test]
    fn bypass_is_exact_integer_arithmetic() {
        // Beyond what an f64 holds exactly in a product with a fraction, but exact as integers.
        let big = 1u64 << 60;
        assert_eq!(mark(1_000, 7, 1.0).source_frame_at(1_000 + big), 7 + big);
    }

    #[test]
    fn a_rate_scales_the_frames_since_the_mark_and_rounds_to_nearest() {
        let m = mark(48_000, 10_000, 1.25);
        assert_eq!(m.source_frame_at(48_000), 10_000);
        assert_eq!(m.source_frame_at(48_000 + 4), 10_005);
        assert_eq!(m.source_frame_at(48_000 + 1), 10_001); // 1.25 -> 1
        assert_eq!(m.source_frame_at(48_000 + 2), 10_003); // 2.5 -> 3
        let slow = mark(0, 100, 0.75);
        assert_eq!(slow.source_frame_at(4), 103);
    }

    #[test]
    fn before_the_mark_starts_the_position_is_its_offset() {
        assert_eq!(mark(5_000, 321, 1.5).source_frame_at(4_000), 321);
        assert_eq!(mark(5_000, 321, 1.0).source_frame_at(0), 321);
    }
}
