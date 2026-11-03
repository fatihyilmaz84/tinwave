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
    /// normally the same item as the mark before it (nothing to announce), see
    /// [`Decoder::restart_for_rate`](crate::decoder).
    Rate,
}

/// "Output frame `start_frame` is frame `offset` of `item`, and from there the source advances
/// `rate` frames per output frame."
///
/// `start_frame` counts frames the renderer has consumed since the engine started; it never
/// resets, so positions stay comparable across seeks and device changes. `offset` and every
/// position derived from it are in **source stream frames**: the track's own timeline at the
/// stream rate, whatever the playback rate (a stream frame of the source is `1 / sample_rate`
/// seconds of the track).
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
}

impl Mark {
    /// The source frame heard at output frame `frame` (at or after `start_frame`).
    pub fn source_frame_at(&self, frame: u64) -> u64 {
        let elapsed = frame.saturating_sub(self.start_frame);
        if self.rate == 1.0 {
            // Bypass stays integer arithmetic: nothing in the float path can touch it.
            return self.offset + elapsed;
        }
        // `as` saturates; an elapsed count that large cannot be a real position.
        self.offset + (elapsed as f64 * self.rate).round() as u64
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
    marks: Mutex<VecDeque<Mark>>,
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
            marks: Mutex::new(VecDeque::new()),
        })
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
