//! The controller's public vocabulary: what it is given and what it reports.

use std::sync::Arc;
use std::time::Duration;

use tinwave_core::{Clock, IdGenerator, RecordingId, SessionId, SourceId, TzOffsetMin};
use tinwave_signals::{InputSource, PlayOrigin, QueueOrigin, RepeatMode, SignalBus};

use super::sink::SinkStats;
use crate::{Position, SourceOpener, Track};

/// A queue entry as the controller needs to know it: the file to play plus the identity and
/// facts every signal about it carries.
///
/// The engine only knows the file. The signals need to say *which recording and source* a skip or
/// a pause was about, how far into the track it happened, and where the entry came from, so the
/// queue layer hands all of that in with the track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    /// The file and its queue identity (unique within one queue).
    pub track: Track,
    /// The musical identity (`recording_id`) every signal about this entry carries.
    pub recording: RecordingId,
    /// The concrete source being played (local file, server item, downloaded copy).
    pub source: SourceId,
    /// Length of the track on the engine's (trimmed) timeline, in milliseconds. Must be above 0:
    /// `Skipped.fraction` and the "within the last 3 %" rule of `Completed` need it, and the
    /// library knows it from the scan.
    pub duration_ms: u64,
    /// Why this entry is in the queue (`Skipped.queue_origin`; also what an auto-advance reports
    /// as the `PlayStarted` origin).
    pub origin: QueueOrigin,
}

/// How a [`Controller::load`](crate::Controller::load) came about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadIntent {
    /// The listener chose this track (double-click, "play album", resume at startup, a remote).
    /// The origin is what `PlayStarted` reports.
    Select(PlayOrigin),
    /// "Next": the queue layer loads the following entry. The track it leaves is `Skipped`.
    Next,
    /// "Previous": the queue layer loads the earlier entry. Emits `PreviousPressed` for the track
    /// it returns to. (Restarting the current track is a `seek` to 0, not a load.)
    Previous,
}

/// Replace the queue and start playing one of its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    /// The new queue.
    pub entries: Vec<QueueEntry>,
    /// Which entry to start with.
    pub index: usize,
    /// First frame to play (nonzero for "resume where I left off").
    pub start_frame: u64,
    /// Why.
    pub intent: LoadIntent,
    /// What drove it.
    pub input: InputSource,
}

/// What the player is doing, for the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    /// No track is current (nothing loaded, stopped, or the queue ended).
    Idle,
    /// A track is current and audible.
    Playing,
    /// A track is current and paused.
    Paused,
}

/// A snapshot the shell reads (the desktop wiring adapts it to `tinwave-context`'s
/// `PlaybackContextSource`; this crate does not depend on that one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackStatus {
    /// The listening session in progress, if any (`SessionStarted` has been emitted for it and
    /// `SessionEnded` has not).
    pub session: Option<SessionId>,
    /// The recording that is current (playing or paused).
    pub recording: Option<RecordingId>,
    /// Its source.
    pub source: Option<SourceId>,
    /// Playing, paused or idle.
    pub state: PlayState,
    /// The repeat mode the listener chose (enforced by the queue layer, T-0023).
    pub repeat: RepeatMode,
}

impl PlaybackStatus {
    /// Whether a listening session is live: a session exists and a track is current. A paused
    /// session counts as live (until 30 minutes of silence end it).
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.session.is_some() && self.state != PlayState::Idle
    }
}

/// What the engine side looks like right now: for the Diagnostics screen and for failure
/// messages that must say why nothing happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerDiagnostics {
    /// Where playback is (see [`Engine::position`](crate::Engine::position)).
    pub position: Option<Position>,
    /// The decoder thread has yet to carry out the last `load`, `seek`, `stop` or
    /// `switch_device` (see [`Engine::command_pending`](crate::Engine::command_pending)).
    pub command_pending: bool,
    /// Events the engine's sink had to coalesce or lose.
    pub sink: SinkStats,
}

/// The local timezone, which the controller needs once per listening session.
///
/// The offset is recorded on the bus (counter days) and, through
/// [`session_started`](Self::session_started), wherever else the shell needs it at the same
/// moment (the signal writer's `set_tz_offset`: this crate cannot see `tinwave-db`).
///
/// Both methods run on the controller thread, in the middle of mapping events. **Implementations
/// must not block** (no waiting on a database lock or on the signal writer): while they wait,
/// playback control stalls.
pub trait TimezoneSource: Send + Sync + std::fmt::Debug {
    /// The local offset from UTC right now.
    fn local_offset(&self) -> TzOffsetMin;

    /// A session is starting with this offset. Called after `SignalBus::set_tz_offset` and before
    /// the session's first event is emitted, so a writer that reads the offset when it first sees
    /// the session has it.
    fn session_started(&self, _offset: TzOffsetMin) {}
}

/// A [`TimezoneSource`] that always reports the same offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedTimezone(pub TzOffsetMin);

impl TimezoneSource for FixedTimezone {
    fn local_offset(&self) -> TzOffsetMin {
        self.0
    }
}

/// Everything a [`Controller`](crate::Controller) is wired to.
pub struct ControllerConfig {
    /// Stream sample rate (stereo `f32`), as for [`EngineConfig`](crate::EngineConfig).
    pub sample_rate: u32,
    /// Output device id, `None` for the system default.
    pub device: Option<String>,
    /// Where track bytes come from.
    pub opener: Arc<dyn SourceOpener>,
    /// The wall clock: engine event times, session timing and gesture windows. Pass the clock the
    /// bus was built with (tests: one `ManualClock` shared by all of them).
    pub clock: Arc<dyn Clock>,
    /// Where signals go.
    pub bus: SignalBus,
    /// Source of `SessionId`s.
    pub ids: IdGenerator,
    /// The local timezone.
    pub timezone: Arc<dyn TimezoneSource>,
    /// The repeat mode restored at launch. It seeds the controller's current and reported modes,
    /// so restoring it emits no `RepeatModeChanged`; only a later change does.
    pub initial_repeat: RepeatMode,
    /// How often the controller thread polls the engine; about 100 ms (see
    /// [`Engine::poll`](crate::Engine::poll)). Only used by
    /// [`ControllerHandle`](crate::ControllerHandle).
    pub poll_interval: Duration,
}

/// The poll cadence the engine documents.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

impl std::fmt::Debug for ControllerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerConfig")
            .field("sample_rate", &self.sample_rate)
            .field("device", &self.device)
            .field("initial_repeat", &self.initial_repeat)
            .field("poll_interval", &self.poll_interval)
            .finish_non_exhaustive()
    }
}
