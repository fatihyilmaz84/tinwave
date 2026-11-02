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
}

/// "Output frame `start_frame` is frame `offset` of `item`."
///
/// `start_frame` counts frames the renderer has consumed since the engine started; it never
/// resets, so positions stay comparable across seeks and device changes.
#[derive(Debug, Clone)]
pub(crate) struct Mark {
    pub seq: u64,
    pub start_frame: u64,
    pub item: Arc<Track>,
    pub offset: u64,
    pub kind: MarkKind,
    /// The item before this one failed mid-decode instead of finishing.
    pub prev_failed: bool,
}

#[derive(Debug)]
pub(crate) struct Shared {
    /// Frames the renderer has taken from the ring. Monotonic, never reset.
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

    /// Lock the mark list. Only the decoder and control threads call this, never the renderer.
    pub fn marks(&self) -> MutexGuard<'_, VecDeque<Mark>> {
        self.marks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
