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
//!   `architecture.md`) and is what the app holds: `Send + Sync`. Commands block for the time the
//!   thread needs to execute them: microseconds, except [`switch_device`](Controller::switch_device),
//!   which opens the output synchronously and can take 100 ms or more (Bluetooth, exclusive
//!   WASAPI); the shell should call it off the UI thread. The thread never waits for the
//!   database, analysis, a context provider or the network, and quitting never waits for the
//!   decoder (a stuck read cannot hang exit).
//! - [`BoundedSink`] is the engine's [`EventSink`](crate::EventSink): bounded, ordered, with a
//!   reserved budget so a flood of position marks can never push out `TrackStarted` / `TrackLeft`.
//!
//! # What is a signal and what is not
//!
//! | `PlaybackEvent` | Signal |
//! |---|---|
//! | `TrackStarted` | `PreviousPressed` for a [`LoadIntent::Previous`] load (opening a session if none); otherwise nothing yet: the track's `PlayStarted` waits for its first audio (see `PositionMark`) |
//! | `PositionMark` | the first one that names the current track: `PlayStarted` (origin, input; `Advance` is `Auto` with the origin derived from the entry's [`QueueOrigin`](tinwave_signals::QueueOrigin); position = where the play began), opening a session if none; later ones: none |
//! | `TrackLeft { Finished }` | `Completed { listened, paused }` (preceded by `PlayStarted` if no mark had named the track yet, provided it advanced past where its stream began: it played to its end; an empty or undecodable file did not) |
//! | `TrackLeft { Replaced }` | `Skipped`, or `Completed` when the track was left with at most 3 % to go; nothing if its audio was never heard |
//! | `TrackLeft { Stopped }` | `Completed` when left with at most 3 % to go, otherwise nothing: stopping is not a preference; nothing if its audio was never heard |
//! | `TrackLeft { Failed }` | **nothing**: a track that could not be played to its end is neither completed nor skipped |
//! | `Paused` / `Resumed` | `Paused { input }` / `Resumed { pause, input }`; a resume after 30 minutes of silence starts a new session first |
//! | `Seeked` | `SeekForward` / `SeekBack`, through the 2 s [`GestureCoalescer`](tinwave_signals::GestureCoalescer) |
//! | `VolumeChanged` | `VolumeChanged`, through the 600 ms coalescer |
//! | `OutputOpened` | none: the stream the engine started with is the starting state, so it seeds the mode `OutputReopened` is compared with |
//! | `OutputReopened` | `ExclusiveModeToggled` when the new stream's mode differs from the previous one |
//! | `QueueEnded` | none (silence starts) |
//! | `QueueEdited` | the signals of the edit it answers (see "Queue edits"): `QueuedNext`, `QueueReordered` or `RemovedFromQueue`, once per item |
//! | `QueueEditRejected` | **nothing**; logged at `warn` and counted in [`ControllerDiagnostics::edits_refused`] |
//! | `Underrun`, `CorruptPackets`, `TrackFailed`, `SeekFailed`, `OutputLost` | none; logged at `warn` |
//!
//! `Paused`, `Resumed`, `Seeked` and `VolumeChanged` with a listener's input, on a play that has not
//! been heard yet while no session is open, open the session first (see "Sessions").
//!
//! Plus [`Controller::set_repeat_mode`] -> `RepeatModeChanged`, [`Controller::queue_later`] ->
//! `QueuedLater`, and the session pair `SessionStarted` / `SessionEnded` (catalog section 8).
//!
//! Not produced here, because the facts live in other layers: `Replayed`, `Favorited`, `Rated`,
//! playlist and metadata edits (library UI), `SystemVolumeChanged`, `MuteToggled`,
//! `OutputDeviceChanged` (`tinwave-context`, ADR 0004), `EqChanged`, `CrossfadeChanged` (no engine
//! feature yet).
//!
//! # Queue edits
//!
//! [`Controller::edit_queue`] (play next, reorder, remove) and [`Controller::queue_later`] change
//! what follows the playing track without interrupting it ([`Engine::replace_after`]). They are
//! preferences, so they are signals: `QueuedNext` and `QueuedLater` (`from_view`, as the caller
//! says), `QueueReordered` (`distance`, signed, as the caller says) and `RemovedFromQueue`
//! (`was_radio_pick` is whether the removed entry's [`QueueOrigin`](tinwave_signals::QueueOrigin)
//! is `Radio`), once per item, with the item's recording and source.
//!
//! - **Only an applied edit is recorded.** The engine's decoder thread has the last word (it reads
//!   up to a second ahead, so in the last second of a track the next item can no longer be
//!   edited). An edit built on a track that is no longer playing fails the call
//!   ([`PlaybackError::StaleQueue`]); one the decoder refuses succeeds and is reported later
//!   (`edits_refused`). Neither records anything. The signals of an accepted edit are emitted
//!   when the decoder reports it applied (`ControllerDiagnostics::edits_pending` counts those
//!   still waiting); `queue_later` has no verdict to wait for and records at once.
//! - **A listener's act.** `Auto` (the queue layer's own refill) applies the edit and records
//!   nothing.
//! - **Sessions.** Like a pause or a seek, an edit needs a session. If none is open and the
//!   playing track has not been heard yet (restored paused at launch), a listener's edit opens
//!   one, with that play's origin (see "Sessions"). Otherwise (nothing loaded, or the play
//!   outlived its session) the edit is applied and not recorded.
//! - The signals carry no `InputSource` (the catalog gives these kinds no `input` field); it only
//!   tells a listener's edit from `Auto`.
//!
//! # Rules the mapping follows
//!
//! - **Section is `Unknown`.** `section`, `section_from`, `section_to` are `Section::UNKNOWN` and
//!   `ms_since_section_start` is 0 until audio analysis (`tinwave-analysis`) can name sections.
//!   Consumers must check `section` before reading the elapsed time.
//! - **`PlayStarted` means the listener heard the track.** It is emitted with the first position
//!   mark that names the track (the engine makes one per 100 ms of heard audio, never while
//!   paused; the first one of a gaplessly advanced track can come sooner than 100 ms in, because
//!   the engine's mark grid does not restart at a gapless hand-off), stamped with that moment, and
//!   carries the position the play began at. A track that fails to open or decode, or is replaced
//!   or stopped before that first mark, leaves no `PlayStarted`, and no `Skipped` / `Completed`
//!   either: an outcome only ever follows its start. A track that plays to its end before any
//!   mark (shorter than 100 ms) is heard: `PlayStarted` then `Completed` at the end. "Plays to its
//!   end" needs audio: leaving at the frame its stream began at (a file with no audio, every
//!   packet undecodable, a seek to the very end) is not a play. Signals about the play that the
//!   listener makes before its `PlayStarted` (a pause, a seek, a volume change) can precede it.
//! - **Every `TrackStarted` has one `TrackLeft`** (engine contract), so every play the controller
//!   opens it closes; gestures are flushed on both, so a seek or volume gesture never spans a
//!   track change. A seek to exactly the end is `Seeked` then `TrackLeft { Finished }`, hence
//!   `SeekForward` then `Completed`.
//! - **Position** (`pos_ds`) is the engine's audio position, never a clock. **Listening time**
//!   (`listened`, `paused`) is wall time between the engine's event timestamps minus the pauses
//!   in between, so it is accurate to one poll (100 ms) and ignores playback rate.
//! - **Sessions** start with the first *heard* play after 30 minutes of silence (`SessionEnded.tracks`
//!   counts heard plays) or, if the listener acts first, with that act: pressing "previous", or a
//!   pause, resume, seek, volume change or queue edit (with a listener's input, not `Auto`) on a play
//!   that has not been heard yet, such as a track restored paused at launch. Such a session carries the play's
//!   origin and can end with `tracks = 0` if the track then fails to open. A track that fails to
//!   open with no listener act opens none. Sessions end after 30 minutes
//!   of silence, measured on the injected clock: silence is the absence of audible playback
//!   (paused, stopped, queue ended). A play that outlives its session (paused for over 30
//!   minutes) is still scored when the listener finally leaves it: that action is the first
//!   play of the next session, so `SessionStarted` (origin of the command: the chosen origin, or
//!   the next entry's for next/previous) comes first, then the `Skipped` / `Completed` of the old
//!   play in the new session, then the new `PlayStarted`. (Only a `stop` in the last 3 % of such
//!   a play has no command origin and is not kept.) `SessionEnded.minutes` runs from the start to the last
//!   audible moment (not including the 30 minutes of silence).
//! - **Gestures and queue edits need a session.** A volume change, a seek or a queue edit with no
//!   session (nothing has played for 30 minutes, or ever; for a seek or an edit: a paused play whose
//!   session has ended) is applied but not recorded.
//! - **Preference changes are not lost for want of a session.** `RepeatModeChanged` and
//!   `ExclusiveModeToggled` made while no session is open are emitted right after the next
//!   `SessionStarted`, if the value still differs from the last one reported. (Queue edits are
//!   many small acts, not one value, and are not deferred.)
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
use tinwave_signals::{InputSource, RepeatMode, View};

use crate::{AudioOutput, Engine, EngineConfig, PlaybackError, Position, Track};
use mapper::{Cx, Mapper};

pub use runtime::ControllerHandle;
pub use sink::{BoundedSink, SinkStats, CRITICAL_CAPACITY, MARK_CAPACITY};
pub use types::{
    ControllerConfig, ControllerDiagnostics, FixedTimezone, LoadIntent, LoadRequest, PlayState,
    PlaybackStatus, QueueEdit, QueueEditRequest, QueueEntry, TimezoneSource, DEFAULT_POLL_INTERVAL,
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
            initial_repeat,
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
        let mut controller = Self {
            engine,
            events,
            mapper: Mapper::new(bus, ids, timezone, sample_rate, initial_repeat),
            clock,
            sample_rate,
            scratch: Vec::with_capacity(64),
            reported: SinkStats::default(),
        };
        // The engine's first event is the stream it opened (`OutputOpened`): take it now, so the
        // starting mode is known before any command.
        controller.drain(Cx::AUTO);
        Ok(controller)
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
            leaving,
        } = request;
        check_durations(&entries)?;
        let tracks: Vec<Track> = entries.iter().map(|e| e.track.clone()).collect();
        // `Engine::load` polls first, and what that poll reports (a gapless hand-off that just
        // happened) is about the old queue. Map it before the queue table is replaced.
        self.poll();
        if let (Some(expected), LoadIntent::Next | LoadIntent::Previous) = (leaving, intent) {
            // The display was a poll behind: the track it showed has already ended and the
            // engine moved on by itself. What the listener pressed "next" on is gone, and the
            // track now playing is not what they meant to leave.
            let playing = self.engine.position().map(|p| p.item);
            if playing != Some(expected) {
                tracing::debug!(
                    expected = expected.0,
                    "next/previous ignored: the track it was pressed on has already ended"
                );
                return Ok(());
            }
        }
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

    /// The listener added entries to the end of the queue ("add to queue"): `QueuedLater` for
    /// each, from `from_view`. As [`append`](Self::append), plus the signal.
    ///
    /// # Errors
    /// As [`append`](Self::append).
    pub fn queue_later(
        &mut self,
        entries: Vec<QueueEntry>,
        from_view: View,
        input: InputSource,
    ) -> Result<(), PlaybackError> {
        self.append(entries.clone())?;
        self.mapper
            .queued_later(self.clock.now(), &entries, from_view, input);
        Ok(())
    }

    /// Change the queue after the playing track without interrupting it: play next, reorder or
    /// remove (see [`QueueEdit`]). The signals are emitted when the engine reports the edit
    /// applied, never for a refused one.
    ///
    /// # Errors
    /// [`PlaybackError::NothingLoaded`] with no current track; [`PlaybackError::StaleQueue`] when
    /// `head` is not the current item (nothing changes, nothing is recorded);
    /// [`PlaybackError::InvalidArgument`] / [`PlaybackError::UnknownItem`] when `edit` does not
    /// match `tail`; otherwise as [`Engine::replace_after`](crate::Engine::replace_after). An edit
    /// that only the decoder thread can refuse (the next track is already started) succeeds here
    /// and shows up as [`ControllerDiagnostics::edits_refused`].
    pub fn edit_queue(&mut self, request: QueueEditRequest) -> Result<(), PlaybackError> {
        let QueueEditRequest {
            head,
            tail,
            edit,
            input,
        } = request;
        check_durations(&tail)?;
        let acts = self.mapper.plan_edit(&edit, &tail)?;
        let tracks: Vec<Track> = tail.iter().map(|e| e.track.clone()).collect();
        // What `Engine::replace_after` polls for is about the queue as it was: map it first.
        self.poll();
        let id = self.engine.replace_after(head, tracks)?;
        self.mapper.extend_queue(&tail);
        self.mapper.expect_edit(id, input, acts);
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
    /// The output is opened synchronously: this can take 100 ms or more (Bluetooth, exclusive
    /// WASAPI) and, through [`ControllerHandle`], blocks the caller meanwhile.
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

    /// Position, pending command, refused queue edits and sink losses in one snapshot.
    #[must_use]
    pub fn diagnostics(&self) -> ControllerDiagnostics {
        ControllerDiagnostics {
            position: self.position(),
            command_pending: self.command_pending(),
            sink: self.sink_stats(),
            edits_pending: self.mapper.edits_pending(),
            edits_refused: self.mapper.edits_refused(),
        }
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

    /// Stops playback, emits what is pending, ends the session and stops the engine, **waiting
    /// for the decoder thread**. For tests and orderly teardown; the app exits through
    /// [`close`](Self::close).
    pub fn shutdown(mut self) {
        self.wind_down();
        self.engine.shutdown();
    }

    /// As [`shutdown`](Self::shutdown) but never waits for the decoder: the engine is only told to
    /// stop (dropping it signals the thread and closes the stream), so a read stuck on a dead
    /// network share cannot hang application exit. This is what [`ControllerHandle`] uses.
    pub fn close(mut self) {
        self.wind_down();
    }

    /// Stop playback, map what that produced, flush gestures and end the session.
    fn wind_down(&mut self) {
        if let Err(error) = self.engine.stop() {
            tracing::warn!(%error, "could not stop playback at shutdown");
        }
        self.drain(Cx::AUTO);
        self.mapper.finish(self.clock.now());
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
