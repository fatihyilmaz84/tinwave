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
//! | `RateChanged` | `RateChanged`, through the 600 ms coalescer (one per [`Controller::set_rate`] call; the stream restart that applies it is not an event) |
//! | `TrackStarted` / `TrackLeft` (not `Failed`, heard) | the play's `RateSession`, a derived value the bus carries to the writer (`SignalBus::emit_derived`), not a bus event; emitted just before the play's outcome event (see "`RateSession`") |
//! | `OutputOpened` / `OutputReopened` | **nothing**, whatever the stream's mode: a reopen at another sample rate (T-0057), an automatic fallback (a device that refuses exclusive mode or a rate) and a stream that is exactly what was asked for are facts about the engine, not preferences. A fallback's reason is logged at `warn`. `ExclusiveModeToggled` has another source, the listener's setting (see "Rules the mapping follows" below) |
//! | `QueueEnded` | none (silence starts) |
//! | `QueueEdited` | the signals of the edit it answers (see "Queue edits"): `QueuedNext`, `QueueReordered` or `RemovedFromQueue`, once per item |
//! | `QueueEditRejected` | **nothing**; logged at `warn` and counted in [`ControllerDiagnostics::edits_refused`] |
//! | `Underrun`, `CorruptPackets`, `TrackFailed`, `SeekFailed`, `OutputLost` | none; logged at `warn` |
//!
//! `Paused`, `Resumed`, `Seeked` and `VolumeChanged` with a listener's input, on a play that has not
//! been heard yet while no session is open, open the session first (see "Sessions").
//!
//! Plus [`Controller::set_repeat_mode`] -> `RepeatModeChanged`, [`Controller::queue_later`] ->
//! `QueuedLater`, the listener's exclusive-mode setting -> `ExclusiveModeToggled`, and the session pair `SessionStarted` / `SessionEnded` (catalog section 8).
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
//! - **Sessions.** An edit is recorded in a session. If none is open and the playing track has not
//!   been heard yet (restored paused at launch), a listener's edit opens one, with that play's
//!   origin (see "Sessions"). Otherwise (nothing loaded, or the play outlived its session) the act
//!   is applied and kept in a buffer of 64 acts (the oldest is dropped when it is full, counted in
//!   [`ControllerDiagnostics::queue_acts_dropped`]) and reported right after the next
//!   `SessionStarted` and after any deferred `RepeatModeChanged` / `ExclusiveModeToggled`, so a
//!   session reads `SessionStarted`, preference changes, queue acts, then the first `PlayStarted`.
//!   Deferred acts carry the session's start time and the recording and source captured at the
//!   act. A kind that is switched off when the act is made buffers nothing, and one switched off
//!   by the time the session starts constructs nothing. Acts still waiting when the controller
//!   closes with no session are lost.
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
//! - **Gestures need a session.** A volume change or a seek with no session (nothing has played
//!   for 30 minutes, or ever; for a seek: a paused play whose session has ended) is applied but
//!   not recorded.
//! - **Preference changes are not lost for want of a session.** `RepeatModeChanged` and
//!   `ExclusiveModeToggled` made while no session is open are emitted right after the next
//!   `SessionStarted`, if the value still differs from the last one reported. Queue acts are
//!   deferred the same way, from a bounded buffer (see "Queue edits").
//! - **`ExclusiveModeToggled` follows the listener's setting, not the stream (T-0067).** The
//!   setting is the output's `AudioOutput::wants_exclusive` (`WasapiControl::set_exclusive`,
//!   `ManualHandle::set_exclusive`); the controller reads it at every drain (each command and each
//!   [`poll`](Controller::poll)) and emits `{ on }` when it differs from the last reading, in the
//!   session or, outside one, after the next `SessionStarted`. So turning exclusive mode off while
//!   a fallback has the stream in shared mode is a toggle, and a fallback or a reopen that changes
//!   the stream's mode is not; a deferred toggle survives any reopen in between. The setting at
//!   launch is the starting state. A toggle and its undo between two readings emit nothing, and a
//!   toggle is stamped with the time of the reading (up to one poll after the change).
//! - **Disabled categories construct nothing**: payloads are built inside the bus's emit closure,
//!   after the toggle check.
//! - **`ctx`** (the context snapshot id) is `None`: attaching snapshots is the context engine's
//!   wiring, not the playback controller's.
//!
//! # Playback rate
//!
//! [`Controller::set_rate`] forwards to the engine's time stretcher (ADR 0006). The engine reports
//! the change as `RateChanged { from, to, position }`; the mapper feeds it, like a volume step,
//! to [`GestureCoalescer::rate`] (600 ms window) with a `RatePayload` and the command's input, so a
//! slider drag is one signal from the first `from` to the last `to`. Like a volume change, it
//! needs a session (or a play not yet heard, which opens one for a listener's input) and is
//! applied but not recorded otherwise.
//!
//! ## `RateSession`
//!
//! The time-weighted rate of a heard play, derived from [`Controller::rate_stats`]: the engine's
//! own counters of frames played and source advanced, read when the play's `TrackStarted` and its
//! `TrackLeft` are handled (the plays of one drain share one reading).
//!
//! - **Delivery.** `RateSession` is a *derived* kind (catalog class D): the bus refuses it as an
//!   event (`SignalsError::NotAnEvent`) and carries it as a `DerivedValue`
//!   (`SignalBus::emit_derived`, ADR 0010). The controller emits it **just before the play's
//!   `Completed` or `Skipped`**, quantized to hundredths (`Rate`), naming the play by the session
//!   and time of its `PlayStarted`; the signals writer stores it in that play's `play` row, in the
//!   same transaction as the outcome that makes the row.
//!   A queue that overflows because the writer stalled loses the new value and counts it in
//!   `SignalBus::dropped(SignalKind::RateSession)`. A play that ends with no outcome event (a stop
//!   before the last 3 %) has a value but no row to hold it: the writer drops it and counts it
//!   (`WriterStats::derived_unmatched`) until stopped plays get a record.
//! - **One per heard play**, when the play ends (`Finished`, `Replaced` or `Stopped`; a play that
//!   outlived its session still gets its value). None for a play that was never heard or that
//!   failed (`Failed`), for a play with no counted audio between its two readings (also a heard
//!   play shorter than one poll, whose start and leave share one reading), and none while the Speed category is
//!   off: the start counters are not even kept if it is off then, so a play that straddles a
//!   toggle (on or off) has none.
//! - **A play spent entirely at 1.0 has one**, equal to 1.0: the catalog row is "per play" with no
//!   exception, and a consumer needs "not rushed" to tell it from "not measured".
//! - **Not exact per play.** The counters are booked when the audio callback takes the frames, not
//!   when they reach the speaker (ADR 0006, "Implementation (T-0027)", note 8), and the controller
//!   reads them up to one poll after the event. A play's edges are off by about one device latency
//!   plus one poll; consecutive plays share their boundary, so nothing is counted twice. The
//!   value is off by at most 2 x (latency + the poll interval, 100 ms by default) x (the
//!   difference between the rates played) / (the play's length), one term per edge; a play at
//!   one rate throughout is exact.
//! - **Exclusive mode.** `wall_frames` counts frames at whichever stream rate played them (T-0057);
//!   the time-weighted rate is a ratio of the two counters and is not affected by a reopen.
//!
//! [`GestureCoalescer::rate`]: tinwave_signals::GestureCoalescer::rate

mod mapper;
mod runtime;
mod session;
mod setting;
mod sink;
mod types;

use std::sync::Arc;

use tinwave_core::Clock;
use tinwave_signals::{InputSource, RepeatMode, View};

use crate::{AudioOutput, Engine, EngineConfig, PlaybackError, Position, Track};
use mapper::{Cx, Mapper};
use setting::ExclusiveSetting;

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
    /// The listener's exclusive-mode setting, read at every drain.
    setting: ExclusiveSetting,
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
        // The setting at launch is the starting state, not a change: nothing is emitted for it.
        let (output, setting) = ExclusiveSetting::split(output);
        let initial_exclusive = setting.wanted();
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
            mapper: Mapper::new(
                bus,
                ids,
                timezone,
                sample_rate,
                initial_repeat,
                initial_exclusive,
            ),
            setting,
            clock,
            sample_rate,
            scratch: Vec::with_capacity(64),
            reported: SinkStats::default(),
        };
        // The engine's first event is the stream it opened (`OutputOpened`, logged only): take it
        // now so the first command's drain starts clean.
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
        // The playing track is not part of what an edit can remove: it keeps playing, so recording
        // it as "removed before it played" would be a rejection of a track being listened to.
        if matches!(&edit, QueueEdit::Remove { items } if items.contains(&head)) {
            return Err(PlaybackError::InvalidArgument(
                "the playing track cannot be removed by a queue edit",
            ));
        }
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

    /// Set the playback rate (0.75 to 1.5, pitch preserved; 1.0 is normal speed).
    ///
    /// # Errors
    /// As [`Engine::set_rate`](crate::Engine::set_rate).
    pub fn set_rate(&mut self, rate: f64, input: InputSource) -> Result<(), PlaybackError> {
        self.engine.set_rate(rate)?;
        self.drain(command(input));
        Ok(())
    }

    /// The playback rate target.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.engine.rate()
    }

    /// What the renderer has played and at which rate (the book `RateSession` is derived from;
    /// the time-weighted rate between two snapshots is
    /// [`RateStats::time_weighted_rate_since`](crate::RateStats::time_weighted_rate_since)).
    #[must_use]
    pub fn rate_stats(&self) -> crate::RateStats {
        self.engine.rate_stats()
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
        self.mapper.status(self.engine.rate_stats().applied_rate)
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

    /// Whether the cadence should be quicker than usual: a command or an output reopen is
    /// waiting for a poll (see [`Engine::poll_soon`](crate::Engine::poll_soon)).
    #[must_use]
    pub fn poll_soon(&self) -> bool {
        self.engine.poll_soon()
    }

    /// Whether the engine has yet to reopen the output at the sample rate of the next track
    /// (exclusive mode; see [`Engine::reopen_pending`](crate::Engine::reopen_pending)).
    #[must_use]
    pub fn reopen_pending(&self) -> bool {
        self.engine.reopen_pending()
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
            queue_acts_dropped: self.mapper.queue_acts_dropped(),
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
        // The listener's setting first: whatever it was when a session opens below is what that
        // session's deferred `ExclusiveModeToggled` reports. A toggle and its undo between two
        // drains (100 ms apart at the usual cadence) are one reading and emit nothing.
        self.mapper
            .set_exclusive_setting(self.clock.now(), self.setting.wanted());
        self.events.drain_into(&mut self.scratch);
        let stats = self.events.stats();
        if stats.overflowed > self.reported.overflowed {
            tracing::error!(
                lost = stats.overflowed - self.reported.overflowed,
                "playback events were lost: the controller fell behind the engine"
            );
        }
        self.reported = stats;
        if self.scratch.is_empty() {
            return;
        }
        // One reading for the whole batch: a gapless hand-off reports the old track's leave and
        // the new track's start together, and they must share the moment so no audio is counted
        // for both plays or neither.
        let rates = self.engine.rate_stats();
        for event in self.scratch.drain(..) {
            self.mapper.handle(event, cx, &rates);
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
