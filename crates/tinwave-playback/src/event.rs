//! Facts the engine reports. The signal layer turns them into T0 signals; the engine itself
//! constructs no signals and knows nothing about the signal bus.

use tinwave_core::UtcMillis;

use crate::{PlaybackError, Position, QueueItemId, StreamInfo};

/// One fact, stamped with the injected clock's wall time.
///
/// Positions inside events come from audio frames, never from the clock.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackEvent {
    /// When the engine produced the event (injected `Clock`).
    pub at: UtcMillis,
    /// What happened.
    pub kind: EventKind,
}

/// Why a track started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartCause {
    /// `load`, which is also what skip, previous and "play this" use.
    Requested,
    /// The previous track ended and the next one followed gaplessly, or the first choice failed
    /// to open and the next queue entry took over.
    Advance,
}

/// Why a track stopped being the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveCause {
    /// It played to its end.
    Finished,
    /// A `load` replaced it (skip, previous, new selection). `frame` is where it was left.
    Replaced,
    /// `stop`.
    Stopped,
    /// It could not be played to its end: it failed to open or decode (a `TrackFailed` event
    /// names the error), or a seek inside it could not be carried out. Never counts as completed.
    Failed,
}

/// The event payloads.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    /// A track became the current one. Input for `PlayStarted`. Every `TrackStarted` is followed by
    /// exactly one `TrackLeft` for the same item before the next `TrackStarted`.
    TrackStarted {
        /// Which track.
        item: QueueItemId,
        /// First frame that will be heard.
        frame: u64,
        /// Why.
        cause: StartCause,
    },
    /// A track stopped being current. Input for `Skipped` (with `Replaced`) and `Completed`
    /// (with `Finished`).
    TrackLeft {
        /// Which track.
        item: QueueItemId,
        /// Frame reached.
        frame: u64,
        /// Why.
        cause: LeaveCause,
    },
    /// Playback paused. Input for `Paused`.
    Paused {
        /// Where, while a track is current; `None` when none is (nothing loaded, stopped, or the
        /// queue ended). Always names a track that has a `TrackStarted`.
        position: Option<Position>,
    },
    /// Playback resumed. Input for `Resumed`.
    Resumed {
        /// Where, while a track is current; `None` when none is (nothing loaded, stopped, or the
        /// queue ended). Always names a track that has a `TrackStarted`.
        position: Option<Position>,
    },
    /// A seek was requested. Input for `SeekForward` / `SeekBack`. Only emitted while a track is
    /// current (it has a `TrackStarted` that is not yet closed); `Engine::seek` is rejected with
    /// `NothingLoaded`, and emits nothing, after the queue ended, after `stop`, or before a `load`.
    Seeked {
        /// Which track.
        item: QueueItemId,
        /// Frame before the seek.
        from_frame: u64,
        /// Requested frame.
        to_frame: u64,
    },
    /// The volume target changed. Input for `VolumeChanged` (the signal layer coalesces gestures).
    VolumeChanged {
        /// Previous `volume_linear`.
        from: f32,
        /// New `volume_linear`.
        to: f32,
        /// Where, while a track is current; `None` when none is (nothing loaded, stopped, or the
        /// queue ended). Always names a track that has a `TrackStarted`.
        position: Option<Position>,
    },
    /// Playback position, once per 100 ms of heard audio while playing.
    PositionMark {
        /// Current position.
        position: Position,
    },
    /// The last queue entry finished and nothing follows.
    QueueEnded,
    /// A track could not be opened or decoded and was skipped.
    TrackFailed {
        /// Which track.
        item: QueueItemId,
        /// Why.
        error: PlaybackError,
    },
    /// A seek could not be carried out; the track is skipped like a failed one.
    SeekFailed {
        /// Which track.
        item: QueueItemId,
        /// Requested frame.
        frame: u64,
        /// Why.
        error: PlaybackError,
    },
    /// The decoder skipped corrupt packets in a track (audible as a small glitch).
    CorruptPackets {
        /// Which track.
        item: QueueItemId,
        /// How many packets were skipped.
        packets: u64,
    },
    /// The renderer ran out of decoded audio while playing. Only reported **after audio has
    /// flowed** since the stream was opened or last flushed: waiting for the first audio of a
    /// `load`, a `seek`, a `switch_device` or a track appended to an ended queue (the decoder is
    /// still opening the file, however long that takes) is buffering, not an underrun, and
    /// produces no event. A stall after audio has been heard is.
    Underrun {
        /// Callbacks that starved since the last report.
        callbacks: u64,
    },
    /// The output stream reported itself lost (device removed or invalidated). The caller decides
    /// whether to `switch_device`.
    OutputLost,
    /// A new output stream was opened (device switch).
    OutputReopened {
        /// Device id as given to `switch_device`; `None` is the system default.
        device: Option<String>,
        /// How the new stream was opened: backend, shared or exclusive mode, sample format,
        /// latency, and why exclusive mode was not used when it was asked for.
        info: StreamInfo,
    },
}

/// Receives events. Called from the control thread (`Engine` methods, `poll`) and from the decoder
/// thread, never from the render callback.
///
/// Implementations must not block: put the event on a bounded queue and return.
pub trait EventSink: Send + Sync + std::fmt::Debug {
    /// Deliver one event.
    fn emit(&self, event: PlaybackEvent);
}
