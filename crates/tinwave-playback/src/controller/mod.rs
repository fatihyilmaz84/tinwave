//! The playback controller: turns what the engine did into what the listener did, as T0 signals.
//!
//! ```text
//! shell/UI --commands (+ InputSource, origin)--> Controller --> Engine --events--> BoundedSink
//!                                                    |                                  |
//!                                                    +-- drain, map, coalesce <---------+
//!                                                    v
//!                                                SignalBus (non-blocking, drops are counted)
//! ```
//!
//! - [`Controller`] is the synchronous core: it owns the [`Engine`](crate::Engine), forwards
//!   commands, drains the engine's events and maps them to signals. Its [`poll`](Controller::poll)
//!   is the 100 ms cadence step. Nothing in it reads the wall clock: times come from the injected
//!   [`Clock`](tinwave_core::Clock), so tests drive it deterministically.
//! - [`ControllerHandle`] runs a `Controller` on its own thread (the playback control thread of
//!   `architecture.md`) and is what the app holds: `Send + Sync`, commands block only for the
//!   microseconds the thread needs to execute them, and the thread never waits for the database,
//!   analysis, a context provider or the network.
//! - [`BoundedSink`] is the engine's [`EventSink`](crate::EventSink): bounded, ordered, with a
//!   reserved budget so a flood of position marks can never push out `TrackStarted` / `TrackLeft`.
//!
//! # What is a signal and what is not
//!
//! | `PlaybackEvent` | Signal |
//! |---|---|
//! | `TrackStarted` | `PlayStarted` (origin, input; `Advance` is `Auto` with the origin derived from the entry's [`QueueOrigin`](tinwave_signals::QueueOrigin)); `PreviousPressed` for a [`LoadIntent::Previous`] load; opens a session if none |
//! | `TrackLeft { Finished }` | `Completed { listened, paused }` |
//! | `TrackLeft { Replaced }` | `Skipped`, or `Completed` when the track was left with at most 3 % to go |
//! | `TrackLeft { Stopped }` | `Completed` when left with at most 3 % to go, otherwise nothing: stopping is not a preference |
//! | `TrackLeft { Failed }` | **nothing**: a track that could not be played to its end is neither completed nor skipped |
//! | `Paused` / `Resumed` | `Paused { input }` / `Resumed { pause, input }`; a resume after 30 minutes of silence starts a new session first |
//! | `Seeked` | `SeekForward` / `SeekBack`, through the 2 s [`GestureCoalescer`](tinwave_signals::GestureCoalescer) |
//! | `VolumeChanged` | `VolumeChanged`, through the 600 ms coalescer |
//! | `OutputReopened` | `ExclusiveModeToggled` when the new stream's mode differs from the previous one |
//! | `QueueEnded` | none (silence starts) |
//! | `PositionMark`, `Underrun`, `CorruptPackets`, `TrackFailed`, `SeekFailed`, `OutputLost` | none; logged at `warn` (marks not at all) |
//!
//! Plus [`Controller::set_repeat_mode`] -> `RepeatModeChanged`, and the session pair
//! `SessionStarted` / `SessionEnded` (catalog section 8).
//!
//! Not produced here, because the facts live in other layers: `Replayed`, `QueuedNext/Later`,
//! `RemovedFromQueue`, `QueueReordered` (queue layer, T-0023), `Favorited`, `Rated`, playlist and
//! metadata edits (library UI), `SystemVolumeChanged`, `MuteToggled`, `OutputDeviceChanged`
//! (`tinwave-context`, ADR 0004), `EqChanged`, `CrossfadeChanged` (no engine feature yet).
//!
//! # Rules the mapping follows
//!
//! - **Section is `Unknown`.** `section`, `section_from`, `section_to` are `Section::UNKNOWN` and
//!   `ms_since_section_start` is 0 until audio analysis (`tinwave-analysis`) can name sections.
//!   Consumers must check `section` before reading the elapsed time.
//! - **Every `TrackStarted` has one `TrackLeft`** (engine contract), so every play the controller
//!   opens it closes; gestures are flushed on both, so a seek or volume gesture never spans a
//!   track change. A seek to exactly the end is `Seeked` then `TrackLeft { Finished }`, hence
//!   `SeekForward` then `Completed`.
//! - **Position** (`pos_ds`) is the engine's audio position, never a clock. **Listening time**
//!   (`listened`, `paused`) is wall time between the engine's event timestamps minus the pauses
//!   in between, so it is accurate to one poll (100 ms) and ignores playback rate.
//! - **Sessions** start with the first play after 30 minutes of silence and end after 30 minutes
//!   of silence, measured on the injected clock: silence is the absence of audible playback
//!   (paused, stopped, queue ended). A play that outlives its session (paused for over 30
//!   minutes) is still scored when the listener finally leaves it: that action is the first
//!   play of the next session, so `SessionStarted` (origin of the command: the chosen origin, or
//!   the next entry's for next/previous) comes first, then the `Skipped` / `Completed` of the old
//!   play in the new session, then the new `PlayStarted`. (Only a `stop` in the last 3 % of such
//!   a play has no command origin and is not kept.) `SessionEnded.minutes` runs from the start to the last
//!   audible moment (not including the 30 minutes of silence).
//! - **Gestures need a session.** A volume change with no session (nothing has played for 30
//!   minutes, or ever) is applied but not recorded.
//! - **Disabled categories construct nothing**: payloads are built inside the bus's emit closure,
//!   after the toggle check.
//! - **`ctx`** (the context snapshot id) is `None`: attaching snapshots is the context engine's
//!   wiring, not the playback controller's.
//!
//! # Where `RateChanged` will go
//!
//! The engine has no playback rate yet (time-stretch, T-0027). When it does, it will report it as
//! a new [`EventKind`](crate::EventKind) variant, and this crate will not compile until
//! `Mapper::handle` has an arm for it: the arm feeds [`GestureCoalescer::rate`] (600 ms window,
//! already implemented in `tinwave-signals`) with a `RatePayload` and the command's input, and
//! the controller gets a `set_rate` command beside `set_volume`.
//!
//! [`GestureCoalescer::rate`]: tinwave_signals::GestureCoalescer::rate

mod mapper;
mod runtime;
mod session;
mod sink;
mod types;

use std::sync::Arc;

use tinwave_core::Clock;
use tinwave_signals::{InputSource, RepeatMode};

use crate::{AudioOutput, Engine, EngineConfig, PlaybackError, Position, Track};
use mapper::{Cx, Mapper};

pub use runtime::ControllerHandle;
pub use sink::{BoundedSink, SinkStats, CRITICAL_CAPACITY, MARK_CAPACITY};
pub use types::{
    ControllerConfig, FixedTimezone, LoadIntent, LoadRequest, PlayState, PlaybackStatus,
    QueueEntry, TimezoneSource, DEFAULT_POLL_INTERVAL,
};

/// The synchronous controller. See the module docs.
pub struct Controller {
    engine: Engine,
    events: Arc<BoundedSink>,
    mapper: Mapper,
    clock: Arc<dyn Clock>,
    sample_rate: u32,
    /// Reused between drains so polling does not allocate.
    scratch: Vec<crate::PlaybackEvent>,
    reported: SinkStats,
}

impl std::fmt::Debug for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Controller")
            .field("engine", &self.engine)
            .field("mapper", &self.mapper)
            .finish_non_exhaustive()
    }
}

impl Controller {
    /// Starts the engine on `output` and wires its events to the signal bus.
    ///
    /// # Errors
    /// As [`Engine::start`].
    pub fn start(
        config: ControllerConfig,
        output: Box<dyn AudioOutput>,
    ) -> Result<Self, PlaybackError> {
        let ControllerConfig {
            sample_rate,
            device,
            opener,
            clock,
            bus,
            ids,
            timezone,
            initial_mode,
            poll_interval: _,
        } = config;
        let events = Arc::new(BoundedSink::new());
        let engine = Engine::start(
            EngineConfig {
                sample_rate,
                device,
                opener,
                sink: events.clone(),
                clock: Arc::clone(&clock),
            },
            output,
        )?;
        Ok(Self {
            engine,
            events,
            mapper: Mapper::new(bus, ids, timezone, sample_rate, initial_mode),
            clock,
            sample_rate,
            scratch: Vec::with_capacity(64),
            reported: SinkStats::default(),
        })
    }

    /// Replace the queue and start playing one of its entries.
    ///
    /// # Errors
    /// [`PlaybackError::InvalidArgument`] for an empty queue, an index outside it, a zero
    /// `duration_ms` or duplicate item ids; otherwise as [`Engine::load`](crate::Engine::load).
    pub fn load(&mut self, request: LoadRequest) -> Result<(), PlaybackError> {
        let LoadRequest {
            entries,
            index,
            start_frame,
            intent,
            input,
        } = request;
        check_durations(&entries)?;
        let tracks: Vec<Track> = entries.iter().map(|e| e.track.clone()).collect();
        // `Engine::load` polls first, and what that poll reports (a gapless hand-off that just
        // happened) is about the old queue. Map it before the queue table is replaced.
        self.poll();
        self.engine.load(tracks, index, start_frame)?;
        self.mapper.set_queue(&entries);
        let cx = Cx {
            input,
            play_origin: match intent {
                LoadIntent::Select(origin) => Some(origin),
                LoadIntent::Next | LoadIntent::Previous => {
                    entries.get(index).map(|e| mapper::play_origin(e.origin))
                }
            },
            previous: intent == LoadIntent::Previous,
        };
        self.drain(cx);
        Ok(())
    }

    /// Add entries after the last one.
    ///
    /// # Errors
    /// As [`load`](Self::load) and [`Engine::append`](crate::Engine::append).
    pub fn append(&mut self, entries: Vec<QueueEntry>) -> Result<(), PlaybackError> {
        check_durations(&entries)?;
        let tracks: Vec<Track> = entries.iter().map(|e| e.track.clone()).collect();
        self.engine.append(tracks)?;
        self.mapper.extend_queue(&entries);
        self.drain(Cx::AUTO);
        Ok(())
    }

    /// Stop playback and clear the queue.
    ///
    /// # Errors
    /// As [`Engine::stop`](crate::Engine::stop).
    pub fn stop(&mut self) -> Result<(), PlaybackError> {
        self.engine.stop()?;
        self.drain(Cx::AUTO);
        Ok(())
    }

    /// Seek the current track to `position_ms`. Gestures within 2 s of each other are one signal
    /// (see the module docs).
    ///
    /// # Errors
    /// [`PlaybackError::NothingLoaded`] with no current track; otherwise as
    /// [`Engine::seek`](crate::Engine::seek).
    pub fn seek_ms(&mut self, position_ms: u64, input: InputSource) -> Result<(), PlaybackError> {
        let frame = position_ms.saturating_mul(u64::from(self.sample_rate)) / 1000;
        self.engine.seek(frame)?;
        self.drain(command(input));
        Ok(())
    }

    /// Pause. No-op when already paused.
    pub fn pause(&mut self, input: InputSource) {
        self.engine.pause();
        self.drain(command(input));
    }

    /// Resume. No-op when not paused.
    pub fn resume(&mut self, input: InputSource) {
        self.engine.resume();
        self.drain(command(input));
    }

    /// Set the software volume (`volume_linear`, 0.0 to 1.0).
    ///
    /// # Errors
    /// As [`Engine::set_volume`](crate::Engine::set_volume).
    pub fn set_volume(
        &mut self,
        volume_linear: f32,
        input: InputSource,
    ) -> Result<(), PlaybackError> {
        self.engine.set_volume(volume_linear)?;
        self.drain(command(input));
        Ok(())
    }

    /// Record the listener's repeat choice and emit `RepeatModeChanged` when it changed. The
    /// controller does not loop anything: enforcing repeat is the queue layer's (T-0023), which
    /// reads [`PlaybackStatus::repeat`].
    pub fn set_repeat_mode(&mut self, mode: RepeatMode) {
        self.mapper.set_repeat(self.clock.now(), mode);
    }

    /// Move to another output device (`None` is the system default).
    ///
    /// # Errors
    /// As [`Engine::switch_device`](crate::Engine::switch_device).
    pub fn switch_device(&mut self, device: Option<String>) -> Result<(), PlaybackError> {
        let result = self.engine.switch_device(device);
        self.drain(Cx::AUTO);
        result
    }

    /// One cadence step; call about every 100 ms. Polls the engine, maps its events, ends
    /// gestures whose window is over and sessions whose silence reached 30 minutes.
    pub fn poll(&mut self) {
        self.engine.poll();
        self.drain(Cx::AUTO);
        self.mapper.tick(self.clock.now());
    }

    /// What the shell needs to know right now.
    #[must_use]
    pub fn status(&self) -> PlaybackStatus {
        self.mapper.status()
    }

    /// Where playback is (see [`Engine::position`](crate::Engine::position)).
    #[must_use]
    pub fn position(&self) -> Option<Position> {
        self.engine.position()
    }

    /// Whether the decoder thread has yet to carry out the last `load`, `seek`, `stop` or
    /// `switch_device` (see [`Engine::command_pending`](crate::Engine::command_pending)).
    #[must_use]
    pub fn command_pending(&self) -> bool {
        self.engine.command_pending()
    }

    /// The software volume target.
    #[must_use]
    pub fn volume(&self) -> f32 {
        self.engine.volume()
    }

    /// Events the engine's sink had to coalesce or lose (see [`BoundedSink`]).
    #[must_use]
    pub fn sink_stats(&self) -> SinkStats {
        self.events.stats()
    }

    /// Stops playback, emits what is pending, ends the session and stops the engine.
    pub fn shutdown(mut self) {
        if let Err(error) = self.engine.stop() {
            tracing::warn!(%error, "could not stop playback at shutdown");
        }
        self.drain(Cx::AUTO);
        self.mapper.finish(self.clock.now());
        self.engine.shutdown();
    }

    /// Maps every event the engine has produced so far. `cx` describes the command that caused
    /// them, if any; events that are not a command's doing carry their own cause.
    fn drain(&mut self, cx: Cx) {
        self.events.drain_into(&mut self.scratch);
        let stats = self.events.stats();
        if stats.overflowed > self.reported.overflowed {
            tracing::error!(
                lost = stats.overflowed - self.reported.overflowed,
                "playback events were lost: the controller fell behind the engine"
            );
        }
        self.reported = stats;
        for event in self.scratch.drain(..) {
            self.mapper.handle(event, cx);
        }
    }
}

fn command(input: InputSource) -> Cx {
    Cx {
        input,
        play_origin: None,
        previous: false,
    }
}

fn check_durations(entries: &[QueueEntry]) -> Result<(), PlaybackError> {
    if entries.iter().all(|e| e.duration_ms > 0) {
        Ok(())
    } else {
        Err(PlaybackError::InvalidArgument(
            "duration_ms must be above 0",
        ))
    }
}
