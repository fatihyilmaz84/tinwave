//! `PlaybackEvent` to signals. Deterministic: no thread, no timer, no engine; the caller feeds it
//! events in order and tells it when time has passed.
//!
//! The `match` in [`Mapper::handle`] has no wildcard arm, so a new [`EventKind`] does not compile
//! until it is decided here what it means for the signals (the table is in the docs of
//! [`controller`](super)).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tinwave_core::{IdGenerator, RecordingId, SessionId, SourceId, UtcMillis};
use tinwave_signals::{
    CompletedPayload, DerivedPayload, DerivedValue, ElapsedDs, EmitOutcome, EventContext,
    ExclusiveModePayload, Fraction, GestureCoalescer, GestureTick, GestureTime, InputSource,
    PausedPayload, PlayOrigin, PlayRef, PlayStartedPayload, PositionDs, QueueOrigin,
    QueueReorderedPayload, QueuedPayload, Rate, RatePayload, RateSessionPayload,
    RemovedFromQueuePayload, RepeatMode, RepeatModePayload, ResumedPayload, Section, SeekPayload,
    SessionEndedPayload, SessionStartedPayload, SignalBus, SignalEvent, SignalKind, SignalPayload,
    SkipPayload, View, Volume, VolumePayload,
};

use super::session::Session;
use super::types::{PlayState, PlaybackStatus, QueueEdit, QueueEntry, TimezoneSource};
use crate::event::{EventKind, LeaveCause, PlaybackEvent, StartCause};
use crate::{EditId, PlaybackError, QueueItemId, RateStats};

/// A track that has played at least this percentage of its length when it is left counts as
/// `Completed` (catalog: "reached end or within the last 3 %").
const COMPLETED_FROM_PERCENT: u64 = 97;

/// Queue edits whose verdict the mapper waits for. The engine's command queue (64) bounds how many
/// can be in flight; this only guards against a decoder that never answers.
const MAX_PENDING_EDITS: usize = 256;

/// Queue acts made with no session to record them in, kept until the next session starts. When
/// it is full the oldest is dropped and counted.
const MAX_DEFERRED_QUEUE_ACTS: usize = 64;

/// What caused the events being handled: the command in progress, or nothing (the engine's own
/// doing, which is `Auto`).
#[derive(Debug, Clone, Copy)]
pub(super) struct Cx {
    pub input: InputSource,
    /// Origin of the play a `load` begins: reported by its `TrackStarted { Requested }` and used
    /// to open a session if the track it leaves had outlived the previous one. `None` for
    /// commands that do not start a play.
    pub play_origin: Option<PlayOrigin>,
    /// The command was "previous".
    pub previous: bool,
}

impl Cx {
    /// No command: the controller's own cadence.
    pub const AUTO: Self = Self {
        input: InputSource::Auto,
        play_origin: None,
        previous: false,
    };
}

/// What a signal about a queue entry says about it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Meta {
    recording: RecordingId,
    source: SourceId,
    duration_ms: u64,
    origin: QueueOrigin,
}

impl From<&QueueEntry> for Meta {
    fn from(entry: &QueueEntry) -> Self {
        Self {
            recording: entry.recording,
            source: entry.source,
            duration_ms: entry.duration_ms,
            origin: entry.origin,
        }
    }
}

/// One queue preference the listener expressed, ready to be turned into a signal once the engine
/// has applied the edit. Self-contained (the queue table may be replaced before the verdict).
#[derive(Debug, Clone, Copy)]
pub(super) enum QueueAct {
    Next(Meta, View),
    Later(Meta, View),
    Reordered(Meta, i16),
    Removed(Meta),
}

impl QueueAct {
    fn meta(self) -> Meta {
        match self {
            Self::Next(meta, _)
            | Self::Later(meta, _)
            | Self::Reordered(meta, _)
            | Self::Removed(meta) => meta,
        }
    }

    fn kind(self) -> SignalKind {
        match self {
            Self::Next(..) => SignalKind::QueuedNext,
            Self::Later(..) => SignalKind::QueuedLater,
            Self::Reordered(..) => SignalKind::QueueReordered,
            Self::Removed(_) => SignalKind::RemovedFromQueue,
        }
    }

    fn payload(self) -> SignalPayload {
        match self {
            Self::Next(_, from_view) => SignalPayload::QueuedNext(QueuedPayload { from_view }),
            Self::Later(_, from_view) => SignalPayload::QueuedLater(QueuedPayload { from_view }),
            Self::Reordered(_, distance) => {
                SignalPayload::QueueReordered(QueueReorderedPayload { distance })
            }
            Self::Removed(meta) => SignalPayload::RemovedFromQueue(RemovedFromQueuePayload {
                was_radio_pick: meta.origin == QueueOrigin::Radio,
            }),
        }
    }
}

/// A queue edit sent to the engine whose verdict has not come back yet.
#[derive(Debug)]
struct PendingEdit {
    id: EditId,
    input: InputSource,
    acts: Vec<QueueAct>,
}

/// What `PlayStarted` will say, kept until the first audio of the track is heard.
#[derive(Debug, Clone, Copy)]
struct PendingStart {
    origin: PlayOrigin,
    input: InputSource,
    /// Where the play began, in milliseconds (nonzero for "resume where I left off").
    start_ms: u64,
}

/// The track that is current. Self-contained: it outlives the queue table, which is replaced when
/// a new queue is loaded, before the old track's `TrackLeft` is handled.
#[derive(Debug, Clone, Copy)]
struct Play {
    item: QueueItemId,
    meta: Meta,
    started_at: UtcMillis,
    /// The frame the current stream of this track began at: the frame it started at, or the target
    /// of the latest seek. A track that is left at a later frame has played some audio.
    from_frame: u64,
    /// `Some` until the track has been heard and `PlayStarted` emitted (see
    /// [`Mapper::heard`]); a track that fails first never emits it.
    unheard: Option<PendingStart>,
    /// Since when the current pause has lasted.
    paused_since: Option<UtcMillis>,
    /// Pauses that already ended.
    paused_ms: i64,
    /// The engine's rate counters when the play began (its `TrackStarted`), for `RateSession`.
    /// `None` when that signal was off then: a play is not measured from a moment the listener had
    /// not agreed to be measured at.
    rate_from: Option<RateStats>,
    /// The session and time of this play's `PlayStarted` event, once it was emitted: what a derived
    /// value names its play by (the `play` row is made of them). `None` before the play is heard,
    /// and for a play whose start was not recorded (its category was off, or no session could be
    /// opened): there is no row to attach a value to.
    started: Option<(SessionId, UtcMillis)>,
}

impl Play {
    fn paused_total(&self, until: UtcMillis) -> i64 {
        let ongoing = self.paused_since.map_or(0, |since| {
            until.as_i64().saturating_sub(since.as_i64()).max(0)
        });
        self.paused_ms.saturating_add(ongoing)
    }
}

/// The event columns beyond session and time.
#[derive(Debug, Clone, Copy, Default)]
struct Scope {
    recording: Option<RecordingId>,
    source: Option<SourceId>,
    pos_ds: Option<u32>,
}

pub(super) struct Mapper {
    bus: SignalBus,
    coalescer: GestureCoalescer,
    ids: IdGenerator,
    timezone: Arc<dyn TimezoneSource>,
    sample_rate: u32,
    table: HashMap<QueueItemId, Meta>,
    edits: VecDeque<PendingEdit>,
    edits_refused: u64,
    /// Listener queue acts waiting for a session (pre-sized, never grows).
    deferred_acts: VecDeque<QueueAct>,
    acts_dropped: u64,
    session: Option<Session>,
    play: Option<Play>,
    /// The engine's pause flag as last reported (it survives a `load`).
    paused: bool,
    repeat: RepeatMode,
    /// The listener's exclusive-mode setting as the controller last read it from the output
    /// (T-0067). It is what the signal follows, never the mode of the stream: a device that
    /// refuses exclusive mode leaves the setting where it was.
    exclusive: bool,
    /// The values last put into a signal (or seen by the bus with their category off). A change
    /// made outside a session is only recorded here; the next session reports it right after
    /// `SessionStarted`, so a preference change is never lost for want of a session.
    reported_repeat: RepeatMode,
    reported_exclusive: bool,
}

impl std::fmt::Debug for Mapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mapper")
            .field("session", &self.session.map(|s| s.id))
            .field("playing", &self.play.map(|p| p.item))
            .field("paused", &self.paused)
            .finish_non_exhaustive()
    }
}

impl Mapper {
    pub fn new(
        bus: SignalBus,
        ids: IdGenerator,
        timezone: Arc<dyn TimezoneSource>,
        sample_rate: u32,
        initial_repeat: RepeatMode,
        initial_exclusive: bool,
    ) -> Self {
        Self {
            coalescer: GestureCoalescer::new(bus.clone()),
            bus,
            ids,
            timezone,
            sample_rate,
            table: HashMap::new(),
            edits: VecDeque::new(),
            edits_refused: 0,
            deferred_acts: VecDeque::with_capacity(MAX_DEFERRED_QUEUE_ACTS),
            acts_dropped: 0,
            session: None,
            play: None,
            paused: false,
            repeat: initial_repeat,
            exclusive: initial_exclusive,
            reported_repeat: initial_repeat,
            reported_exclusive: initial_exclusive,
        }
    }

    /// The queue the engine was given: what each item is, for the events that name it.
    pub fn set_queue<'a>(&mut self, entries: impl IntoIterator<Item = &'a QueueEntry>) {
        self.table = entries
            .into_iter()
            .map(|e| (e.track.id, Meta::from(e)))
            .collect();
    }

    /// More entries after the last one.
    pub fn extend_queue<'a>(&mut self, entries: impl IntoIterator<Item = &'a QueueEntry>) {
        self.table
            .extend(entries.into_iter().map(|e| (e.track.id, Meta::from(e))));
    }

    /// Check a queue edit against what it claims to be and work out what the listener expressed.
    /// Nothing changes yet: the signals wait for the engine's verdict ([`expect_edit`](Self::expect_edit)).
    ///
    /// # Errors
    /// [`PlaybackError::InvalidArgument`] when the claim does not match `tail`;
    /// [`PlaybackError::UnknownItem`] for a moved or removed item the mapper has never seen.
    pub fn plan_edit(
        &self,
        edit: &QueueEdit,
        tail: &[QueueEntry],
    ) -> Result<Vec<QueueAct>, PlaybackError> {
        match edit {
            QueueEdit::PlayNext { items, from_view } => {
                let leads = !items.is_empty()
                    && items.len() <= tail.len()
                    && items.iter().zip(tail).all(|(id, e)| *id == e.track.id);
                if !leads {
                    return Err(PlaybackError::InvalidArgument(
                        "play next items must be the first entries of the tail",
                    ));
                }
                Ok(tail[..items.len()]
                    .iter()
                    .map(|e| QueueAct::Next(Meta::from(e), *from_view))
                    .collect())
            }
            QueueEdit::Reorder { item, distance } => {
                if *distance == 0 {
                    return Err(PlaybackError::InvalidArgument(
                        "a reorder moves an item by at least one place",
                    ));
                }
                let entry = tail
                    .iter()
                    .find(|e| e.track.id == *item)
                    .ok_or(PlaybackError::UnknownItem(*item))?;
                Ok(vec![QueueAct::Reordered(Meta::from(entry), *distance)])
            }
            QueueEdit::Remove { items } => {
                if items.is_empty() {
                    return Err(PlaybackError::InvalidArgument("nothing was removed"));
                }
                items
                    .iter()
                    .map(|id| {
                        if tail.iter().any(|e| e.track.id == *id) {
                            return Err(PlaybackError::InvalidArgument(
                                "a removed item is still in the tail",
                            ));
                        }
                        self.table
                            .get(id)
                            .map(|meta| QueueAct::Removed(*meta))
                            .ok_or(PlaybackError::UnknownItem(*id))
                    })
                    .collect()
            }
        }
    }

    /// A queue edit was sent to the engine: its signals are emitted when the engine reports that it
    /// was applied, and never if it is refused.
    pub fn expect_edit(&mut self, id: EditId, input: InputSource, acts: Vec<QueueAct>) {
        if self.edits.len() >= MAX_PENDING_EDITS {
            tracing::error!(
                "the engine never answered {MAX_PENDING_EDITS} queue edits; forgetting the oldest"
            );
            self.edits.pop_front();
        }
        self.edits.push_back(PendingEdit { id, input, acts });
    }

    /// The listener added `entries` to the end of the queue (the engine has them already).
    pub fn queued_later(
        &mut self,
        at: UtcMillis,
        entries: &[QueueEntry],
        from_view: View,
        input: InputSource,
    ) {
        let acts: Vec<QueueAct> = entries
            .iter()
            .map(|e| QueueAct::Later(Meta::from(e), from_view))
            .collect();
        self.record_queue_acts(at, input, &acts);
    }

    /// Emits the signals of queue preferences. A listener's act only: `Auto` is the queue layer
    /// refilling, not a preference. Needs a session; a play that has not been heard yet and has none
    /// opens one, like any other act of the listener on it (`open_session_for_unheard`). With no
    /// session and no such play (nothing loaded, or the play outlived its session) the act waits in
    /// a bounded buffer and is reported right after the next `SessionStarted`
    /// ([`report_queue_acts`](Self::report_queue_acts)): a preference is not lost for want of a
    /// session. A kind that is switched off buffers nothing.
    fn record_queue_acts(&mut self, at: UtcMillis, input: InputSource, acts: &[QueueAct]) {
        self.expire(at);
        if input == InputSource::Auto {
            return;
        }
        let cx = Cx {
            input,
            play_origin: None,
            previous: false,
        };
        for act in acts {
            let kind = act.kind();
            if !self.bus.enabled(kind) {
                continue;
            }
            self.open_session_for_unheard(at, cx, kind);
            if self.session.is_some() {
                self.send_queue_act(at, *act);
            } else {
                if self.deferred_acts.len() >= MAX_DEFERRED_QUEUE_ACTS {
                    self.deferred_acts.pop_front();
                    self.acts_dropped = self.acts_dropped.saturating_add(1);
                }
                self.deferred_acts.push_back(*act);
            }
        }
    }

    fn send_queue_act(&self, at: UtcMillis, act: QueueAct) {
        let meta = act.meta();
        let scope = Scope {
            recording: Some(meta.recording),
            source: Some(meta.source),
            pos_ds: None,
        };
        self.send(act.kind(), at, scope, || act.payload());
    }

    /// Queue acts made while there was no session are reported now, right after `SessionStarted`
    /// and the deferred preference changes, oldest first. They carry the time the session started
    /// (not the time of the act) and the recording and source captured at the act. A kind that has
    /// been switched off since constructs nothing.
    fn report_queue_acts(&mut self, at: UtcMillis) {
        while let Some(act) = self.deferred_acts.pop_front() {
            self.send_queue_act(at, act);
        }
    }

    /// Queue acts dropped because more than 64 were waiting for a session.
    pub fn queue_acts_dropped(&self) -> u64 {
        self.acts_dropped
    }

    /// Queue edits waiting for the decoder thread's verdict.
    pub fn edits_pending(&self) -> usize {
        self.edits.len()
    }

    /// Queue edits the decoder thread refused after the engine call succeeded.
    pub fn edits_refused(&self) -> u64 {
        self.edits_refused
    }

    fn edit_applied(&mut self, at: UtcMillis, id: EditId) {
        let Some(index) = self.edits.iter().position(|e| e.id == id) else {
            return;
        };
        if let Some(edit) = self.edits.remove(index) {
            self.record_queue_acts(at, edit.input, &edit.acts);
        }
    }

    /// `rate` is the engine's applied rate: the mapper does not know it.
    pub fn status(&self, rate: f64) -> PlaybackStatus {
        PlaybackStatus {
            session: self.session.map(|s| s.id),
            recording: self.play.map(|p| p.meta.recording),
            source: self.play.map(|p| p.meta.source),
            state: match (self.play, self.paused) {
                (None, _) => PlayState::Idle,
                (Some(_), true) => PlayState::Paused,
                (Some(_), false) => PlayState::Playing,
            },
            repeat: self.repeat,
            rate,
        }
    }

    /// Time has passed: end gestures whose window of silence is over and sessions whose 30
    /// minutes are.
    pub fn tick(&mut self, now: UtcMillis) {
        let _ = self.coalescer.flush_due();
        self.expire(now);
    }

    /// The app is closing: emit what is pending and end the session.
    pub fn finish(&mut self, now: UtcMillis) {
        let _ = self.coalescer.flush_all();
        self.end_session(now);
    }

    /// The listener chose a repeat mode. The mapper only records it and says so; enforcing it is
    /// the queue layer's job.
    pub fn set_repeat(&mut self, at: UtcMillis, mode: RepeatMode) {
        self.expire(at);
        if mode == self.repeat {
            return;
        }
        self.repeat = mode;
        // Outside a session the change waits for the next one (see `report_preferences`).
        if self.session.is_some() {
            self.report_repeat(at);
        }
    }

    /// The listener's exclusive-mode setting, as the output reports it now (T-0067). Called with
    /// every reading; a value equal to the last one does nothing. Like
    /// [`set_repeat`](Self::set_repeat) it records the change and, in a session, reports it; outside
    /// one the next session does. Nothing about the stream the engine opened feeds this.
    pub fn set_exclusive_setting(&mut self, at: UtcMillis, on: bool) {
        if on == self.exclusive {
            return;
        }
        self.expire(at);
        self.exclusive = on;
        if self.session.is_some() {
            self.report_exclusive(at);
        }
    }

    /// `rates` are the engine's rate counters as of the drain this event is part of (the same for
    /// every event of one drain): the book `RateSession` is derived from.
    pub fn handle(&mut self, event: PlaybackEvent, cx: Cx, rates: &RateStats) {
        let at = event.at;
        self.expire(at);
        match event.kind {
            EventKind::TrackStarted { item, frame, cause } => {
                self.track_started(at, item, frame, cause, cx, rates);
            }
            EventKind::TrackLeft { item, frame, cause } => {
                self.track_left(at, item, frame, cause, cx, rates);
            }
            EventKind::Paused { position } => {
                self.paused_event(at, position.map(|p| p.position_ms), cx);
            }
            EventKind::Resumed { position } => {
                self.resumed_event(at, position.map(|p| p.position_ms), cx);
            }
            EventKind::Seeked {
                item,
                from_frame,
                to_frame,
            } => self.seeked(at, item, from_frame, to_frame, cx),
            EventKind::VolumeChanged { from, to, position } => {
                self.volume_changed(at, from, to, position.map(|p| (p.item, p.position_ms)), cx);
            }
            EventKind::RateChanged { from, to, position } => {
                self.rate_changed(at, from, to, position.map(|p| (p.item, p.position_ms)), cx);
            }
            // Audio is being heard: that is when the track's `PlayStarted` is due. Nothing else
            // in the catalog is derived from it. (Listening time comes from the event times,
            // positions from the events that carry one.)
            EventKind::PositionMark { position } => self.heard(at, position.item),
            EventKind::QueueEnded => {
                if let Some(session) = &mut self.session {
                    session.set_audible(at, false);
                }
            }
            EventKind::TrackFailed { item, error } => {
                tracing::warn!(item = item.0, %error, "track failed; the queue moved on");
            }
            EventKind::SeekFailed { item, frame, error } => {
                tracing::warn!(item = item.0, frame, %error, "seek failed; the track was skipped");
            }
            EventKind::QueueEdited { edit, .. } => self.edit_applied(at, edit),
            EventKind::QueueEditRejected { edit, head, error } => {
                self.edits.retain(|e| e.id != edit);
                self.edits_refused = self.edits_refused.saturating_add(1);
                tracing::warn!(head = head.0, %error, "queue edit refused; the queue is unchanged");
            }
            EventKind::CorruptPackets { item, packets } => {
                tracing::warn!(item = item.0, packets, "decoder skipped corrupt packets");
            }
            EventKind::Underrun { callbacks } => {
                tracing::warn!(callbacks, "audio underrun");
            }
            EventKind::OutputLost => {
                tracing::warn!("audio output lost");
            }
            EventKind::OutputOpened { info, .. } | EventKind::OutputReopened { info, .. } => {
                Self::output_stream(&info);
            }
        }
    }

    // --- sessions ---------------------------------------------------------------------------

    /// Ends the session if its silence has lasted 30 minutes by `now`.
    fn expire(&mut self, now: UtcMillis) {
        if let Some(end) = self.session.as_ref().and_then(|s| s.ended_by(now)) {
            let _ = self.coalescer.flush_all();
            self.end_session(end);
        }
    }

    fn end_session(&mut self, at: UtcMillis) {
        let Some(session) = self.session.take() else {
            return;
        };
        let audible_until = session.silence_since.unwrap_or(at);
        let payload = SessionEndedPayload {
            tracks: u16::try_from(session.tracks).unwrap_or(u16::MAX),
            minutes: session.minutes(audible_until),
        };
        self.send_in(
            &session,
            SignalKind::SessionEnded,
            at,
            Scope::default(),
            || SignalPayload::SessionEnded(payload),
        );
    }

    /// A play begins (a track started, or playback resumed): opens a session if there is none.
    fn ensure_session(&mut self, at: UtcMillis, origin: PlayOrigin) {
        if self.session.is_some() {
            return;
        }
        let id = match SessionId::generate(&mut self.ids) {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(%error, "cannot create a session id; signals are off until the next play");
                return;
            }
        };
        let mut session = Session::new(id);
        // A session opened while paused (the listener sought or changed the volume of a track
        // restored paused, or pressed "previous" while paused) starts in silence: otherwise it
        // would never end by silence, and a play hours later would join it.
        session.set_audible(at, !self.paused);
        let tz = self.timezone.local_offset();
        self.bus.set_tz_offset(tz);
        self.timezone.session_started(tz);
        self.send_in(
            &session,
            SignalKind::SessionStarted,
            at,
            Scope::default(),
            || {
                SignalPayload::SessionStarted(SessionStartedPayload {
                    start_origin: origin,
                })
            },
        );
        self.session = Some(session);
        self.report_preferences(at);
        self.report_queue_acts(at);
    }

    /// Preference changes made while there was no session are reported now, right after
    /// `SessionStarted`.
    fn report_preferences(&mut self, at: UtcMillis) {
        if self.repeat != self.reported_repeat {
            self.report_repeat(at);
        }
        if self.exclusive != self.reported_exclusive {
            self.report_exclusive(at);
        }
    }

    /// Emits `RepeatModeChanged` for the current mode. When the change was made outside a session
    /// (see [`report_preferences`](Self::report_preferences)) the event is deferred, and then it
    /// carries the time the session started (not the time of the change) and whichever recording
    /// is current at that moment, which need not be the track the listener was on when they
    /// changed it: the track that is being heard (or that "previous" returned to) when that opens
    /// the session, the paused track when a resume does, none when a stale play is being left
    /// (it has been taken already). Catalog section 8, "Session boundaries".
    fn report_repeat(&mut self, at: UtcMillis) {
        let mode = self.repeat;
        self.reported_repeat = mode;
        let scope = self.play_scope(None);
        self.send(SignalKind::RepeatModeChanged, at, scope, || {
            SignalPayload::RepeatModeChanged(RepeatModePayload { mode })
        });
    }

    /// Emits `ExclusiveModeToggled` for the current setting (deferred the same way as
    /// [`report_repeat`](Self::report_repeat) when the change was made outside a session).
    fn report_exclusive(&mut self, at: UtcMillis) {
        let on = self.exclusive;
        self.reported_exclusive = on;
        self.send(
            SignalKind::ExclusiveModeToggled,
            at,
            Scope::default(),
            || SignalPayload::ExclusiveModeToggled(ExclusiveModePayload { on }),
        );
    }

    fn set_audible(&mut self, at: UtcMillis, audible: bool) {
        if let Some(session) = &mut self.session {
            session.set_audible(at, audible);
        }
    }

    // --- tracks -----------------------------------------------------------------------------

    fn track_started(
        &mut self,
        at: UtcMillis,
        item: QueueItemId,
        frame: u64,
        cause: StartCause,
        cx: Cx,
        rates: &RateStats,
    ) {
        let _ = self.coalescer.flush_all();
        let Some(meta) = self.table.get(&item).copied() else {
            // Benign when a gapless hand-off in the old queue lands between `Controller::load`'s
            // pre-poll and `Engine::load`'s own poll and the new queue lacks that item: the play
            // is simply not recorded. A genuinely unknown item would be a queue-layer bug, but
            // the two cannot be told apart here, so this is `debug`, not `error`.
            tracing::debug!(
                item = item.0,
                "a track started that is not in the queue table; it produces no signals"
            );
            return;
        };
        let (origin, input) = match cause {
            StartCause::Requested => (
                cx.play_origin.unwrap_or_else(|| play_origin(meta.origin)),
                cx.input,
            ),
            StartCause::Advance => (play_origin(meta.origin), InputSource::Auto),
        };
        let start_ms = self.frame_ms(frame);
        self.play = Some(Play {
            item,
            meta,
            started_at: at,
            from_frame: frame,
            unheard: Some(PendingStart {
                origin,
                input,
                start_ms,
            }),
            paused_since: self.paused.then_some(at),
            paused_ms: 0,
            // The play begins here, not at its `PlayStarted`: that waits for 100 ms of heard audio,
            // which this would leave out of the average.
            rate_from: self.bus.enabled(SignalKind::RateSession).then_some(*rates),
            started: None,
        });
        self.set_audible(at, !self.paused);

        // `PlayStarted` (and the session it opens, if none is open) waits for the first audio.
        // Pressing "previous" is the listener's act whether or not the track then plays.
        if cause == StartCause::Requested && cx.previous {
            self.ensure_session(at, origin);
            let scope = self.scope_of(Some(meta), Some(start_ms));
            self.send(SignalKind::PreviousPressed, at, scope, || {
                SignalPayload::PreviousPressed
            });
        }
    }

    /// Audio of `item` is being heard (a position mark names it): emits its `PlayStarted`, the
    /// first time only. The engine makes a mark per 100 ms of heard audio, never while paused and
    /// never before the output has played the stream's first frames (the first mark of a gaplessly
    /// advanced track can come sooner than 100 ms in: its mark grid is not restarted), so a track
    /// that fails to open, or is left before any mark names it, is not a play.
    fn heard(&mut self, at: UtcMillis, item: QueueItemId) {
        let Some(play) = self.play.as_mut().filter(|p| p.item == item) else {
            return;
        };
        let Some(start) = play.unheard.take() else {
            return;
        };
        let meta = play.meta;
        let started = self.play_started(at, meta, start);
        if let Some(play) = self.play.as_mut().filter(|p| p.item == item) {
            play.started = started;
        }
    }

    /// Emits `PlayStarted`. The session, if none is open, opens here: it begins with the first
    /// play that is heard.
    ///
    /// Returns the session and the time the event was emitted with, which is how the play's row is
    /// named later; `None` when no event is emitted (no session, or the signal is off).
    fn play_started(
        &mut self,
        at: UtcMillis,
        meta: Meta,
        start: PendingStart,
    ) -> Option<(SessionId, UtcMillis)> {
        let PendingStart {
            origin,
            input,
            start_ms,
        } = start;
        self.ensure_session(at, origin);
        if let Some(session) = &mut self.session {
            session.tracks = session.tracks.saturating_add(1);
        }
        let scope = self.scope_of(Some(meta), Some(start_ms));
        let started = self
            .session
            .as_ref()
            .filter(|_| self.bus.enabled(SignalKind::PlayStarted))
            .map(|session| (session.id, session.stamp(at)));
        self.send(SignalKind::PlayStarted, at, scope, || {
            SignalPayload::PlayStarted(PlayStartedPayload { origin, input })
        });
        started
    }

    fn track_left(
        &mut self,
        at: UtcMillis,
        item: QueueItemId,
        frame: u64,
        cause: LeaveCause,
        cx: Cx,
        rates: &RateStats,
    ) {
        let _ = self.coalescer.flush_all();
        let Some(mut play) = self.play.take().filter(|p| p.item == item) else {
            // Its start was never handled (unknown item), so there is nothing to close.
            return;
        };
        self.set_audible(at, false);
        if let Some(start) = play.unheard {
            if cause != LeaveCause::Finished || frame <= play.from_frame {
                // Never heard: it failed to open or decode, was left before its first audio, or
                // "finished" without a single frame past where its stream began (an empty or
                // truncated file, every packet undecodable, a seek to the very end). Not a play,
                // so neither completed nor skipped.
                return;
            }
            // Played to its end, so it was heard (a track shorter than one position mark): the
            // play starts and ends now.
            play.started = self.play_started(at, play.meta, start);
        }
        let position_ms = self.frame_ms(frame);
        let near_end = position_ms.saturating_mul(100)
            >= play.meta.duration_ms.saturating_mul(COMPLETED_FROM_PERCENT);
        let completed = match cause {
            LeaveCause::Finished => true,
            LeaveCause::Replaced | LeaveCause::Stopped => near_end,
            // Neither completed nor skipped: the track could not be played to its end.
            LeaveCause::Failed => return,
        };
        // Before the outcome event, so that the value and the outcome reach the writer in one
        // transaction: the value takes the next `seq`, the drain hands events and values over in
        // `seq` order, and the play-end event makes the writer commit at once. The writer applies
        // the value to the record that outcome makes. A stop in the middle of a track is heard and
        // leaves no outcome, but it has a rate (the writer has no record for it and drops it).
        self.derive_rate_session(at, &play, rates);
        if completed || cause == LeaveCause::Replaced {
            // A play that outlived its session (paused over 30 minutes) is left by a real user
            // action: that action is the first play of the next session, so the session opens
            // here, with the origin of the command that caused it, and the outcome lands in it.
            // (`stop` has no origin: a stale stop at the very end is the one outcome not kept.)
            if let Some(origin) = cx.play_origin {
                self.ensure_session(at, origin);
            }
        }
        let scope = self.scope_of(Some(play.meta), Some(position_ms));
        if completed {
            let wall = at.as_i64().saturating_sub(play.started_at.as_i64()).max(0);
            let paused = play.paused_total(at);
            let listened = wall.saturating_sub(paused).max(0);
            self.send(SignalKind::Completed, at, scope, || {
                SignalPayload::Completed(CompletedPayload {
                    listened_ds: PositionDs::from_ms(listened.unsigned_abs()),
                    paused_ds: PositionDs::from_ms(paused.unsigned_abs()),
                })
            });
        } else if cause == LeaveCause::Replaced {
            let fraction = fraction_of(position_ms, play.meta.duration_ms);
            self.send(SignalKind::Skipped, at, scope, || {
                SignalPayload::Skipped(SkipPayload {
                    fraction,
                    input: cx.input,
                    section: Section::UNKNOWN,
                    queue_origin: play.meta.origin,
                })
            });
        }
        // `Stopped` before the last 3 % is the listener ending playback: not a preference.
    }

    /// `RateSession` of a heard play that is being left: the time-weighted rate between the
    /// counters at its `TrackStarted` and now. One per heard play that was not cut short by a
    /// failure, outcome or not, including a play that never left 1.0 (the catalog row has no such
    /// exception, and a consumer needs the 1.0 to tell "not rushed" from "not measured"). None when
    /// the signal was off at the start or is off now, when no audio was played in between (nothing
    /// to average), or when the play's start was not recorded (no record to attach it to).
    ///
    /// It is a derived value, so it goes to the bus as a [`DerivedValue`], not as an event: the
    /// writer stores it in the play's record, quantized to hundredths.
    fn derive_rate_session(&self, at: UtcMillis, play: &Play, rates: &RateStats) {
        if !self.bus.enabled(SignalKind::RateSession) {
            return;
        }
        let Some(from) = &play.rate_from else {
            return;
        };
        let Some((session, started)) = play.started else {
            tracing::debug!(
                item = play.item.0,
                "a play with no recorded start has no record; no RateSession"
            );
            return;
        };
        let Some(time_weighted_rate) = rates.time_weighted_rate_since(from) else {
            tracing::debug!(
                item = play.item.0,
                "no audio was counted over a heard play; no RateSession"
            );
            return;
        };
        // Within 0.75 to 1.5 by construction, so the narrowing loses nothing the hundredths keep.
        let rate = match Rate::from_rate(time_weighted_rate as f32) {
            Ok(rate) => rate,
            Err(error) => {
                tracing::warn!(%error, "a play's rate is out of range; no RateSession");
                return;
            }
        };
        let outcome = self
            .bus
            .emit_derived(SignalKind::RateSession, |stamp| DerivedValue {
                id: stamp.id,
                ts: at,
                play: PlayRef {
                    session,
                    recording: play.meta.recording,
                    started,
                },
                payload: DerivedPayload::RateSession(RateSessionPayload { rate }),
            });
        match outcome {
            Ok(EmitOutcome::Queued | EmitOutcome::Disabled) => {}
            // The bus has counted it (`SignalBus::dropped`, shown in Diagnostics).
            Ok(EmitOutcome::Dropped) => {
                tracing::warn!(kind = %SignalKind::RateSession, "derived value lost to bus overflow");
            }
            Err(error) => {
                tracing::error!(kind = %SignalKind::RateSession, %error, "derived value rejected by the bus");
            }
        }
    }

    // --- pause, seek, volume ----------------------------------------------------------------

    /// The listener paused, resumed, sought or changed the volume of a play whose audio has not
    /// been heard yet, and no session is open (a track restored paused at launch is the common
    /// case): that act is the first thing the listener did, so the session begins with it, with
    /// the play's origin. Without it the act would go unrecorded. Not for the controller's own
    /// (`Auto`) commands, which are not the listener's, nor for an act whose signal (`kind`) is
    /// turned off: a session would then open for something that is not recorded.
    fn open_session_for_unheard(&mut self, at: UtcMillis, cx: Cx, kind: SignalKind) {
        if self.session.is_some() || cx.input == InputSource::Auto || !self.bus.enabled(kind) {
            return;
        }
        if let Some(start) = self.play.and_then(|p| p.unheard) {
            self.ensure_session(at, start.origin);
        }
    }

    fn paused_event(&mut self, at: UtcMillis, position_ms: Option<u64>, cx: Cx) {
        self.paused = true;
        self.open_session_for_unheard(at, cx, SignalKind::Paused);
        self.set_audible(at, false);
        let Some(play) = &mut self.play else {
            return;
        };
        play.paused_since.get_or_insert(at);
        let meta = play.meta;
        let scope = self.scope_of(Some(meta), position_ms);
        self.send(SignalKind::Paused, at, scope, || {
            SignalPayload::Paused(PausedPayload { input: cx.input })
        });
    }

    fn resumed_event(&mut self, at: UtcMillis, position_ms: Option<u64>, cx: Cx) {
        self.paused = false;
        let Some(play) = &mut self.play else {
            return;
        };
        let pause_ms = play
            .paused_since
            .take()
            .map_or(0, |since| at.as_i64().saturating_sub(since.as_i64()).max(0));
        play.paused_ms = play.paused_ms.saturating_add(pause_ms);
        let meta = play.meta;
        // Resuming after 30 minutes of silence is the first play of a new session. (A play that
        // was never heard has no session of its own to resume: the listener's act opens one with
        // the play's origin, as `PlayStarted` would.)
        if play.unheard.is_none() {
            self.ensure_session(at, PlayOrigin::Resume);
        } else {
            self.open_session_for_unheard(at, cx, SignalKind::Resumed);
        }
        self.set_audible(at, true);
        let scope = self.scope_of(Some(meta), position_ms);
        self.send(SignalKind::Resumed, at, scope, || {
            SignalPayload::Resumed(ResumedPayload {
                pause_ds: PositionDs::from_ms(pause_ms.unsigned_abs()),
                input: cx.input,
            })
        });
    }

    fn seeked(&mut self, at: UtcMillis, item: QueueItemId, from_frame: u64, to_frame: u64, cx: Cx) {
        if let Some(play) = self.play.as_mut().filter(|p| p.item == item) {
            play.from_frame = to_frame;
        }
        let kind = if to_frame >= from_frame {
            SignalKind::SeekForward
        } else {
            SignalKind::SeekBack
        };
        self.open_session_for_unheard(at, cx, kind);
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let meta = self.meta_of(item);
        let from = PositionDs::from_ms(self.frame_ms(from_frame));
        let to = PositionDs::from_ms(self.frame_ms(to_frame));
        let context = EventContext {
            session: session.id,
            recording: meta.map(|m| m.recording),
            source: meta.map(|m| m.source),
            pos_ds: Some(to.ds()),
            ctx: None,
        };
        let _ = self.coalescer.seek(|| GestureTick {
            context,
            payload: SeekPayload {
                from_ds: from,
                to_ds: to,
                section_from: Section::UNKNOWN,
                section_to: Section::UNKNOWN,
                input: cx.input,
            },
        });
    }

    fn volume_changed(
        &mut self,
        at: UtcMillis,
        from: f32,
        to: f32,
        position: Option<(QueueItemId, u64)>,
        cx: Cx,
    ) {
        if !self.bus.enabled(SignalKind::VolumeChanged) {
            return;
        }
        self.open_session_for_unheard(at, cx, SignalKind::VolumeChanged);
        let Some(session) = self.session.as_ref() else {
            // No session means nothing has played for 30 minutes (or ever): not listening.
            return;
        };
        let (Ok(from), Ok(to)) = (Volume::from_linear(from), Volume::from_linear(to)) else {
            tracing::error!("the engine reported a volume that is not a number; not recorded");
            return;
        };
        let meta = position.and_then(|(item, _)| self.meta_of(item));
        let position_ms = position.map(|(_, ms)| ms);
        let context = EventContext {
            session: session.id,
            recording: meta.map(|m| m.recording),
            source: meta.map(|m| m.source),
            pos_ds: position_ms.map(|ms| PositionDs::from_ms(ms).ds()),
            ctx: None,
        };
        let _ = self.coalescer.volume(|| GestureTick {
            context,
            payload: VolumePayload {
                from,
                to,
                gesture: GestureTime::from_ms(0),
                input: cx.input,
                section: Section::UNKNOWN,
                since_track_start: ElapsedDs::from_ms(position_ms.unwrap_or(0)),
                since_section_start: ElapsedDs::from_ds(0),
            },
        });
    }

    /// A playback-rate step. Like a volume step it is a gesture: the coalescer keeps the first
    /// `from` and the last `to` of the steps within 600 ms of each other.
    fn rate_changed(
        &mut self,
        at: UtcMillis,
        from: f64,
        to: f64,
        position: Option<(QueueItemId, u64)>,
        cx: Cx,
    ) {
        if !self.bus.enabled(SignalKind::RateChanged) {
            return;
        }
        self.open_session_for_unheard(at, cx, SignalKind::RateChanged);
        let Some(session) = self.session.as_ref() else {
            // No session means nothing has played for 30 minutes (or ever): not listening.
            return;
        };
        // The signal stores hundredths of a rate; the engine's range is well inside it.
        let (Ok(from), Ok(to)) = (Rate::from_rate(from as f32), Rate::from_rate(to as f32)) else {
            tracing::error!("the engine reported a rate outside the signal's range; not recorded");
            return;
        };
        let meta = position.and_then(|(item, _)| self.meta_of(item));
        let position_ms = position.map(|(_, ms)| ms);
        let context = EventContext {
            session: session.id,
            recording: meta.map(|m| m.recording),
            source: meta.map(|m| m.source),
            pos_ds: position_ms.map(|ms| PositionDs::from_ms(ms).ds()),
            ctx: None,
        };
        let _ = self.coalescer.rate(|| GestureTick {
            context,
            payload: RatePayload {
                from,
                to,
                input: cx.input,
                section: Section::UNKNOWN,
            },
        });
    }

    /// A stream the engine opened or reopened. The stream is a fact about the engine, not a
    /// preference (an automatic fallback, a reopen at another sample rate), so it is never a
    /// signal: `ExclusiveModeToggled` follows the listener's setting
    /// ([`set_exclusive_setting`](Self::set_exclusive_setting)). A fallback is only logged.
    fn output_stream(info: &crate::StreamInfo) {
        if let Some(reason) = &info.fallback {
            tracing::warn!(%reason, "the output stream is not what was asked for");
        }
    }

    // --- helpers ----------------------------------------------------------------------------

    fn frame_ms(&self, frame: u64) -> u64 {
        frame.saturating_mul(1000) / u64::from(self.sample_rate)
    }

    /// The queue entry an event names: the current track, else the table.
    fn meta_of(&self, item: QueueItemId) -> Option<Meta> {
        self.play
            .filter(|p| p.item == item)
            .map(|p| p.meta)
            .or_else(|| self.table.get(&item).copied())
    }

    fn scope_of(&self, meta: Option<Meta>, position_ms: Option<u64>) -> Scope {
        Scope {
            recording: meta.map(|m| m.recording),
            source: meta.map(|m| m.source),
            pos_ds: position_ms.map(|ms| PositionDs::from_ms(ms).ds()),
        }
    }

    /// The columns for an event about whatever is current (no position).
    fn play_scope(&self, position_ms: Option<u64>) -> Scope {
        self.scope_of(self.play.map(|p| p.meta), position_ms)
    }

    /// Emits in the current session; nothing happens outside a session.
    fn send(
        &self,
        kind: SignalKind,
        at: UtcMillis,
        scope: Scope,
        payload: impl FnOnce() -> SignalPayload,
    ) {
        if let Some(session) = &self.session {
            self.send_in(session, kind, at, scope, payload);
        }
    }

    /// Emits in `session`. The payload is only built if the kind is collected.
    fn send_in(
        &self,
        session: &Session,
        kind: SignalKind,
        at: UtcMillis,
        scope: Scope,
        payload: impl FnOnce() -> SignalPayload,
    ) {
        let ts = session.stamp(at);
        let outcome = self.bus.emit_with(kind, |stamp| SignalEvent {
            id: stamp.id,
            ts,
            session: session.id,
            recording: scope.recording,
            source: scope.source,
            pos_ds: scope.pos_ds,
            payload: payload(),
            ctx: None,
        });
        match outcome {
            // Off: by design nothing was built.
            Ok(EmitOutcome::Queued | EmitOutcome::Disabled) => {}
            // The bus has counted it (`SignalBus::dropped`, shown in Diagnostics).
            Ok(EmitOutcome::Dropped) => tracing::warn!(%kind, "signal lost to bus overflow"),
            Err(error) => tracing::error!(%kind, %error, "signal rejected by the bus"),
        }
    }
}

/// Fraction of the track heard, per-255, saturating at the end.
fn fraction_of(position_ms: u64, duration_ms: u64) -> Fraction {
    let raw = (u128::from(position_ms) * 255 + u128::from(duration_ms) / 2)
        / u128::from(duration_ms.max(1));
    Fraction::from_raw(u8::try_from(raw.min(255)).unwrap_or(u8::MAX))
}

/// The `PlayStarted` origin of an entry nobody asked for by name (auto-advance, next, previous).
pub(super) fn play_origin(origin: QueueOrigin) -> PlayOrigin {
    match origin {
        QueueOrigin::Manual | QueueOrigin::Shuffle => PlayOrigin::Queue,
        QueueOrigin::Radio => PlayOrigin::Radio,
        QueueOrigin::Album => PlayOrigin::Album,
        QueueOrigin::Playlist => PlayOrigin::Playlist,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_is_rounded_and_saturates() {
        assert_eq!(fraction_of(0, 1000).raw(), 0);
        assert_eq!(fraction_of(500, 1000).raw(), 128);
        assert_eq!(fraction_of(1000, 1000).raw(), 255);
        assert_eq!(fraction_of(5000, 1000).raw(), 255);
        assert_eq!(
            fraction_of(10, 0).raw(),
            255,
            "an unknown length cannot panic"
        );
    }

    #[test]
    fn every_queue_origin_maps_to_a_play_origin() {
        let all = [
            QueueOrigin::Manual,
            QueueOrigin::Radio,
            QueueOrigin::Album,
            QueueOrigin::Playlist,
            QueueOrigin::Shuffle,
        ];
        let mapped: Vec<_> = all.iter().map(|&o| play_origin(o)).collect();
        assert_eq!(
            mapped,
            [
                PlayOrigin::Queue,
                PlayOrigin::Radio,
                PlayOrigin::Album,
                PlayOrigin::Playlist,
                PlayOrigin::Queue
            ]
        );
    }

    // --- RateSession ------------------------------------------------------------------------

    use tinwave_core::{DeviceId, ManualClock, SeededRng, TzOffsetMin};
    use tinwave_signals::{
        BusConfig, CounterRow, DerivedPayload, SignalCategory, SignalDrain, SignalSettings,
        SignalSink, SignalsError,
    };

    use super::super::types::FixedTimezone;
    use crate::{Position, Track};

    const T0: i64 = 1_793_577_600_000;

    /// What the drain handed over, in the order it did.
    #[derive(Default)]
    struct Collected {
        events: Vec<SignalEvent>,
        derived: Vec<DerivedValue>,
    }

    impl SignalSink for Collected {
        fn write(&mut self, events: &[SignalEvent], _: &[CounterRow]) -> Result<(), SignalsError> {
            self.events.extend_from_slice(events);
            Ok(())
        }

        fn write_with_derived(
            &mut self,
            events: &[SignalEvent],
            counters: &[CounterRow],
            derived: &[DerivedValue],
        ) -> Result<(), SignalsError> {
            self.write(events, counters)?;
            self.derived.extend_from_slice(derived);
            Ok(())
        }
    }

    struct Rig {
        mapper: Mapper,
        settings: Arc<SignalSettings>,
        bus: SignalBus,
        drain: SignalDrain,
        collected: Collected,
    }

    fn rig() -> Rig {
        let clock = ManualClock::new(UtcMillis::new(T0));
        let settings = Arc::new(SignalSettings::new());
        let device = DeviceId::from_parts(UtcMillis::new(T0), [9; 10]).expect("device id");
        let (bus, drain) =
            SignalBus::channel(BusConfig::new(device), clock.clone(), settings.clone())
                .expect("bus");
        let mut mapper = Mapper::new(
            bus.clone(),
            IdGenerator::new(clock, SeededRng::new(7)),
            Arc::new(FixedTimezone(TzOffsetMin::new(0).expect("offset"))),
            48_000,
            RepeatMode::Off,
            false,
        );
        let entries: Vec<QueueEntry> = (1..=3).map(entry).collect();
        mapper.set_queue(&entries);
        Rig {
            mapper,
            settings,
            bus,
            drain,
            collected: Collected::default(),
        }
    }

    fn entry(id: u64) -> QueueEntry {
        QueueEntry {
            track: Track {
                id: QueueItemId(id),
                path: "unused.wav".into(),
                gapless: None,
            },
            recording: recording_of(id),
            source: SourceId::from_parts(UtcMillis::new(T0), [id as u8; 10]).expect("source"),
            duration_ms: 10_000,
            origin: QueueOrigin::Manual,
        }
    }

    fn recording_of(id: u64) -> RecordingId {
        RecordingId::from_parts(UtcMillis::new(T0), [id as u8; 10]).expect("recording")
    }

    /// Counters after `wall` frames played, which advanced `source` frames of source.
    fn stats(wall: u64, source: f64) -> RateStats {
        RateStats {
            wall_frames: wall,
            source_micro_frames: (source * 1e6) as u64,
            effective_rate: 1.0,
            applied_rate: 1.0,
            flushes: 0,
        }
    }

    impl Rig {
        fn event(&mut self, kind: EventKind, rates: &RateStats) {
            let at = UtcMillis::new(T0);
            self.mapper
                .handle(PlaybackEvent { at, kind }, Cx::AUTO, rates);
        }

        fn start(&mut self, item: u64, rates: &RateStats) {
            self.event(
                EventKind::TrackStarted {
                    item: QueueItemId(item),
                    frame: 0,
                    cause: StartCause::Requested,
                },
                rates,
            );
        }

        /// The first position mark: the play is heard.
        fn hear(&mut self, item: u64, rates: &RateStats) {
            self.event(
                EventKind::PositionMark {
                    position: Position {
                        item: QueueItemId(item),
                        frame: 4_800,
                        position_ms: 100,
                    },
                },
                rates,
            );
        }

        fn leave(&mut self, item: u64, cause: LeaveCause, rates: &RateStats) {
            self.event(
                EventKind::TrackLeft {
                    item: QueueItemId(item),
                    frame: 48_000,
                    cause,
                },
                rates,
            );
        }

        /// Everything the bus has accepted so far, drained into `collected`.
        fn pump(&mut self) -> &Collected {
            while self.drain.pump(&mut self.collected).expect("pump").more {}
            &self.collected
        }

        /// The rate sessions emitted since the last call, as `(recording, rate)`.
        fn sessions(&mut self) -> Vec<(RecordingId, f64)> {
            self.pump();
            std::mem::take(&mut self.collected.derived)
                .into_iter()
                .map(|v| {
                    let DerivedPayload::RateSession(p) = v.payload;
                    (v.play.recording, f64::from(p.rate.rate()))
                })
                .collect()
        }
    }

    #[test]
    fn a_heard_play_is_measured_between_its_start_and_its_leave() {
        let mut rig = rig();
        // Counters from earlier audio are not part of this play.
        rig.start(1, &stats(100_000, 90_000.0));
        rig.hear(1, &stats(104_800, 94_800.0));
        // 48 000 frames at 1.25 since the start.
        rig.leave(1, LeaveCause::Stopped, &stats(148_000, 150_000.0));
        let sessions = rig.sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].0, recording_of(1));
        assert!((sessions[0].1 - 1.25).abs() < 1e-12, "{}", sessions[0].1);
    }

    #[test]
    fn plays_in_one_batch_share_their_boundary() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        // A gapless hand-off: one reading serves the leave of 1 and the start of 2.
        let boundary = stats(48_000, 48_000.0);
        rig.leave(1, LeaveCause::Finished, &boundary);
        rig.start(2, &boundary);
        rig.hear(2, &stats(52_800, 48_000.0 + 4_800.0 * 1.5));
        rig.leave(
            2,
            LeaveCause::Finished,
            &stats(96_000, 48_000.0 + 48_000.0 * 1.5),
        );
        let sessions = rig.sessions();
        assert_eq!(
            sessions.iter().map(|s| s.0).collect::<Vec<_>>(),
            [recording_of(1), recording_of(2)]
        );
        assert!((sessions[0].1 - 1.0).abs() < 1e-12);
        assert!((sessions[1].1 - 1.5).abs() < 1e-12);
    }

    #[test]
    fn a_play_that_never_left_normal_speed_has_one_at_exactly_one() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        rig.leave(1, LeaveCause::Replaced, &stats(48_000, 48_000.0));
        assert_eq!(rig.sessions(), [(recording_of(1), 1.0)]);
    }

    #[test]
    fn a_play_left_before_it_was_heard_has_none() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.leave(1, LeaveCause::Replaced, &stats(2_000, 2_500.0));
        rig.start(2, &stats(2_000, 2_500.0));
        rig.leave(2, LeaveCause::Stopped, &stats(9_000, 11_000.0));
        assert_eq!(rig.sessions(), []);
    }

    #[test]
    fn a_heard_play_that_then_failed_has_none() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 6_000.0));
        rig.leave(1, LeaveCause::Failed, &stats(30_000, 37_500.0));
        assert_eq!(rig.sessions(), []);
    }

    #[test]
    fn a_track_shorter_than_one_mark_is_heard_when_it_finishes() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.leave(1, LeaveCause::Finished, &stats(2_000, 2_500.0));
        assert_eq!(rig.sessions(), [(recording_of(1), 1.25)]);
    }

    #[test]
    fn a_play_with_no_counted_audio_has_none() {
        let mut rig = rig();
        rig.start(1, &stats(500, 500.0));
        rig.hear(1, &stats(500, 500.0));
        rig.leave(1, LeaveCause::Stopped, &stats(500, 500.0));
        assert_eq!(rig.sessions(), []);
    }

    #[test]
    fn nothing_is_derived_with_the_speed_category_off() {
        let mut rig = rig();
        rig.settings.set_category(SignalCategory::Speed, false);
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        rig.leave(1, LeaveCause::Stopped, &stats(48_000, 60_000.0));
        assert_eq!(rig.sessions(), []);
    }

    #[test]
    fn a_play_that_straddles_the_speed_toggle_has_none() {
        let mut rig = rig();
        // Switched off during the play.
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        rig.settings.set_category(SignalCategory::Speed, false);
        rig.leave(1, LeaveCause::Stopped, &stats(48_000, 60_000.0));
        // Switched on during the play: it was not measured from its start.
        rig.start(2, &stats(48_000, 60_000.0));
        rig.hear(2, &stats(52_800, 66_000.0));
        rig.settings.set_category(SignalCategory::Speed, true);
        rig.leave(2, LeaveCause::Stopped, &stats(96_000, 120_000.0));
        assert_eq!(rig.sessions(), []);
        // The next play, started with it on, is measured.
        rig.start(3, &stats(96_000, 120_000.0));
        rig.hear(3, &stats(100_800, 126_000.0));
        rig.leave(3, LeaveCause::Stopped, &stats(144_000, 180_000.0));
        assert_eq!(rig.sessions(), [(recording_of(3), 1.25)]);
    }

    #[test]
    fn the_value_is_quantized_to_hundredths() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        // 1.2349 plays as 1.23, 1.2351 as 1.24: the rate type's resolution.
        rig.leave(1, LeaveCause::Replaced, &stats(48_000, 48_000.0 * 1.2349));
        rig.start(2, &stats(48_000, 48_000.0 * 1.2349));
        rig.hear(2, &stats(52_800, 48_000.0 * 1.2349 + 4_800.0));
        rig.leave(
            2,
            LeaveCause::Replaced,
            &stats(96_000, 48_000.0 * 1.2349 + 48_000.0 * 1.2351),
        );
        let got = rig.sessions();
        assert_eq!(got.len(), 2);
        assert!((got[0].1 - 1.23).abs() < 1e-6, "{}", got[0].1);
        assert!((got[1].1 - 1.24).abs() < 1e-6, "{}", got[1].1);
    }

    /// The value names its play by the session and time of the `PlayStarted` event it follows,
    /// and takes the `seq` just before the outcome event, so that the two are committed together
    /// and the writer finds the record the outcome makes.
    #[test]
    fn the_value_names_its_play_and_precedes_the_outcome_event() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        rig.leave(1, LeaveCause::Replaced, &stats(48_000, 48_000.0 * 1.5));
        let collected = rig.pump();
        let started = collected
            .events
            .iter()
            .find(|e| e.kind() == SignalKind::PlayStarted)
            .expect("PlayStarted");
        let outcome = collected
            .events
            .iter()
            .find(|e| e.kind() == SignalKind::Skipped)
            .expect("Skipped");
        let [value] = collected.derived.as_slice() else {
            panic!("one value, got {:?}", collected.derived);
        };
        assert_eq!(value.play.session, started.session);
        assert_eq!(value.play.recording, started.recording.expect("recording"));
        assert_eq!(value.play.started, started.ts);
        assert!(
            value.id.seq < outcome.id.seq && value.id.seq > started.id.seq,
            "between the start and the outcome"
        );
        assert_eq!(value.kind(), SignalKind::RateSession);
    }

    /// With the start of the play not recorded there is no record to attach a value to: none is
    /// emitted (the Queue and library category holds `PlayStarted`).
    #[test]
    fn a_play_whose_start_was_not_recorded_has_no_value() {
        let mut rig = rig();
        rig.settings
            .set_category(SignalCategory::QueueLibrary, false);
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        rig.leave(1, LeaveCause::Replaced, &stats(48_000, 60_000.0));
        assert_eq!(rig.sessions(), []);
        assert_eq!(rig.bus.derived_queued_total(), 0);
    }

    /// A value is emitted for a play that has no outcome event (stopped mid-track): it is the
    /// writer that drops it for want of a record, not the controller.
    #[test]
    fn a_stopped_play_still_emits_its_value() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        rig.leave(1, LeaveCause::Stopped, &stats(24_000, 30_000.0));
        let collected = rig.pump();
        assert!(!collected
            .events
            .iter()
            .any(|e| matches!(e.kind(), SignalKind::Completed | SignalKind::Skipped)));
        assert_eq!(rig.sessions(), [(recording_of(1), 1.25)]);
    }

    /// A value still waiting in the bus when the listener switches Speed off is not delivered.
    #[test]
    fn a_value_queued_before_speed_was_switched_off_is_not_delivered() {
        let mut rig = rig();
        rig.start(1, &stats(0, 0.0));
        rig.hear(1, &stats(4_800, 4_800.0));
        rig.leave(1, LeaveCause::Replaced, &stats(48_000, 60_000.0));
        assert_eq!(rig.bus.derived_queued_total(), 1);
        rig.settings.set_category(SignalCategory::Speed, false);
        assert_eq!(rig.sessions(), []);
        assert_eq!(rig.bus.derived_withdrawn(), 1);
    }
}
