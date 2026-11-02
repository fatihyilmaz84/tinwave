//! `PlaybackEvent` to signals. Deterministic: no thread, no timer, no engine; the caller feeds it
//! events in order and tells it when time has passed.
//!
//! The `match` in [`Mapper::handle`] has no wildcard arm, so a new [`EventKind`] does not compile
//! until it is decided here what it means for the signals (the table is in the docs of
//! [`controller`](super)).

use std::collections::HashMap;
use std::sync::Arc;

use tinwave_core::{IdGenerator, RecordingId, SessionId, SourceId, UtcMillis};
use tinwave_signals::{
    CompletedPayload, ElapsedDs, EmitOutcome, EventContext, ExclusiveModePayload, Fraction,
    GestureCoalescer, GestureTick, GestureTime, InputSource, PausedPayload, PlayOrigin,
    PlayStartedPayload, PositionDs, QueueOrigin, RepeatMode, RepeatModePayload, ResumedPayload,
    Section, SeekPayload, SessionEndedPayload, SessionStartedPayload, SignalBus, SignalEvent,
    SignalKind, SignalPayload, SkipPayload, Volume, VolumePayload,
};

use super::session::Session;
use super::types::{PlayState, PlaybackStatus, QueueEntry, TimezoneSource};
use crate::event::{EventKind, LeaveCause, PlaybackEvent, StartCause};
use crate::{QueueItemId, StreamMode};

/// A track that has played at least this percentage of its length when it is left counts as
/// `Completed` (catalog: "reached end or within the last 3 %").
const COMPLETED_FROM_PERCENT: u64 = 97;

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
struct Meta {
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
    /// `Some` until the track has been heard and `PlayStarted` emitted (see
    /// [`Mapper::heard`]); a track that fails first never emits it.
    unheard: Option<PendingStart>,
    /// Since when the current pause has lasted.
    paused_since: Option<UtcMillis>,
    /// Pauses that already ended.
    paused_ms: i64,
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
    session: Option<Session>,
    play: Option<Play>,
    /// The engine's pause flag as last reported (it survives a `load`).
    paused: bool,
    mode: StreamMode,
    repeat: RepeatMode,
    /// The values last put into a signal (or seen by the bus with their category off). A change
    /// made outside a session is only recorded here; the next session reports it right after
    /// `SessionStarted`, so a preference change is never lost for want of a session.
    reported_repeat: RepeatMode,
    reported_mode: StreamMode,
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
    ) -> Self {
        Self {
            coalescer: GestureCoalescer::new(bus.clone()),
            bus,
            ids,
            timezone,
            sample_rate,
            table: HashMap::new(),
            session: None,
            play: None,
            paused: false,
            // Until the engine's `OutputOpened` (the first event it produces) says otherwise.
            mode: StreamMode::Shared,
            repeat: initial_repeat,
            reported_repeat: initial_repeat,
            reported_mode: StreamMode::Shared,
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

    pub fn status(&self) -> PlaybackStatus {
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

    pub fn handle(&mut self, event: PlaybackEvent, cx: Cx) {
        let at = event.at;
        self.expire(at);
        match event.kind {
            EventKind::TrackStarted { item, frame, cause } => {
                self.track_started(at, item, frame, cause, cx);
            }
            EventKind::TrackLeft { item, frame, cause } => {
                self.track_left(at, item, frame, cause, cx);
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
            } => self.seeked(item, from_frame, to_frame, cx),
            EventKind::VolumeChanged { from, to, position } => {
                self.volume_changed(from, to, position.map(|p| (p.item, p.position_ms)), cx);
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
            EventKind::CorruptPackets { item, packets } => {
                tracing::warn!(item = item.0, packets, "decoder skipped corrupt packets");
            }
            EventKind::Underrun { callbacks } => {
                tracing::warn!(callbacks, "audio underrun");
            }
            EventKind::OutputLost => {
                tracing::warn!("audio output lost");
            }
            EventKind::OutputOpened { info, .. } => self.output_opened(&info),
            EventKind::OutputReopened { info, .. } => self.output_reopened(at, &info),
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
        let session = Session::new(id);
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
    }

    /// Preference changes made while there was no session are reported now, right after
    /// `SessionStarted`.
    fn report_preferences(&mut self, at: UtcMillis) {
        if self.repeat != self.reported_repeat {
            self.report_repeat(at);
        }
        if self.mode != self.reported_mode {
            self.report_mode(at);
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

    fn report_mode(&mut self, at: UtcMillis) {
        let on = self.mode == StreamMode::Exclusive;
        self.reported_mode = self.mode;
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
            unheard: Some(PendingStart {
                origin,
                input,
                start_ms,
            }),
            paused_since: self.paused.then_some(at),
            paused_ms: 0,
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
    /// first time only. Marks come once per 100 ms of heard audio, never while paused and never
    /// before the output has played the track's first frames, so a track that fails to open, or
    /// is left before its first 100 ms were heard, is not a play.
    fn heard(&mut self, at: UtcMillis, item: QueueItemId) {
        let Some(play) = self.play.as_mut().filter(|p| p.item == item) else {
            return;
        };
        let Some(start) = play.unheard.take() else {
            return;
        };
        let meta = play.meta;
        self.play_started(at, meta, start);
    }

    /// Emits `PlayStarted`. The session, if none is open, opens here: it begins with the first
    /// play that is heard.
    fn play_started(&mut self, at: UtcMillis, meta: Meta, start: PendingStart) {
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
        self.send(SignalKind::PlayStarted, at, scope, || {
            SignalPayload::PlayStarted(PlayStartedPayload { origin, input })
        });
    }

    fn track_left(
        &mut self,
        at: UtcMillis,
        item: QueueItemId,
        frame: u64,
        cause: LeaveCause,
        cx: Cx,
    ) {
        let _ = self.coalescer.flush_all();
        let Some(play) = self.play.take().filter(|p| p.item == item) else {
            // Its start was never handled (unknown item), so there is nothing to close.
            return;
        };
        self.set_audible(at, false);
        if let Some(start) = play.unheard {
            if cause != LeaveCause::Finished {
                // Never heard: it failed to open or decode, or was left before its first audio.
                // Not a play, so neither completed nor skipped.
                return;
            }
            // Played to its end, so it was heard (a track shorter than one position mark, or a
            // seek to the very end before the first mark): the play starts and ends now.
            self.play_started(at, play.meta, start);
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

    // --- pause, seek, volume ----------------------------------------------------------------

    fn paused_event(&mut self, at: UtcMillis, position_ms: Option<u64>, cx: Cx) {
        self.paused = true;
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
        // was never heard has none yet: its `PlayStarted` opens one with the right origin.)
        if play.unheard.is_none() {
            self.ensure_session(at, PlayOrigin::Resume);
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

    fn seeked(&mut self, item: QueueItemId, from_frame: u64, to_frame: u64, cx: Cx) {
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

    fn volume_changed(&mut self, from: f32, to: f32, position: Option<(QueueItemId, u64)>, cx: Cx) {
        if !self.bus.enabled(SignalKind::VolumeChanged) {
            return;
        }
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

    /// The stream the engine started with: the starting state, not a change, so it seeds both the
    /// current and the reported mode and emits nothing.
    fn output_opened(&mut self, info: &crate::StreamInfo) {
        if let Some(reason) = &info.fallback {
            tracing::warn!(%reason, "the output stream is not what was asked for");
        }
        self.mode = info.mode;
        self.reported_mode = info.mode;
    }

    fn output_reopened(&mut self, at: UtcMillis, info: &crate::StreamInfo) {
        if let Some(reason) = &info.fallback {
            tracing::warn!(%reason, "the output stream is not what was asked for");
        }
        self.mode = info.mode;
        // Outside a session the change waits for the next one (see `report_preferences`).
        if self.mode != self.reported_mode && self.session.is_some() {
            self.report_mode(at);
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
}
