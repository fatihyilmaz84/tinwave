//! The control side of the engine: commands in, [`PlaybackEvent`]s out.

use std::collections::HashSet;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;

use rtrb::RingBuffer;
use tinwave_core::Clock;

use crate::decoder::{Cmd, Decoder};
use crate::event::{EventKind, EventSink, LeaveCause, PlaybackEvent, StartCause};
use crate::output::{AudioOutput, OpenRequest, OutputStream, StreamInfo, StreamMode};
use crate::reader::SourceOpener;
use crate::shared::{Mark, MarkKind, Shared};
use crate::stretch::{MAX_RATE, MIN_RATE};
use crate::types::CHANNELS;
use crate::{EditId, PlaybackError, Position, QueueItemId, Renderer, Track};

/// Seconds of audio the ring holds between the decoder and the callback.
const RING_SECONDS: usize = 1;
/// Position marks per second of heard audio.
const POSITION_MARKS_PER_SECOND: u32 = 10;
/// Commands that may wait for the decoder thread before calls fail with [`PlaybackError::Busy`].
const COMMAND_QUEUE: usize = 64;
/// Fades at a rate change are this many milliseconds long (ADR 0006).
const RATE_FADE_MS: u32 = 5;

/// What the renderer has played, and at what rate: the engine's own account of playback speed,
/// read from counters the audio callback keeps (no guess from the UI).
///
/// The counters only grow. `RateSession.time_weighted_rate` of one play is
/// [`time_weighted_rate_since`](Self::time_weighted_rate_since) between a snapshot taken when the
/// play began and one taken when it ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateStats {
    /// Frames of audio the renderer has played (wall-clock frames at the stream rate), fades
    /// included. Silence the callback produced for want of audio is not counted. In exclusive
    /// mode the stream rate changes with the files (the output is reopened at each file's rate),
    /// so this counts frames of whatever rate played them; the time-weighted rate, a ratio of
    /// frames played to frames advanced, is not affected.
    pub wall_frames: u64,
    /// Source frames those frames advanced through the tracks, in millionths: a frame played at
    /// rate 1.25 adds 1,250,000.
    pub source_micro_frames: u64,
    /// Rate (source frames per output frame) of the last block that carried audio.
    pub effective_rate: f64,
    /// The rate of the audio the decoder is producing now. It trails
    /// [`Engine::rate`] by the time a change takes to be applied.
    pub applied_rate: f64,
    /// Rate flushes the decoder has carried out: how often the stream restarted for a new rate.
    pub flushes: u64,
}

impl RateStats {
    /// Source frames advanced, as a number of frames.
    #[must_use]
    pub fn source_frames(&self) -> f64 {
        self.source_micro_frames as f64 / 1e6
    }

    /// Source frames per played frame between `earlier` and `self`: `None` when no audio was
    /// played in between.
    #[must_use]
    pub fn time_weighted_rate_since(&self, earlier: &Self) -> Option<f64> {
        let wall = self.wall_frames.checked_sub(earlier.wall_frames)?;
        let micro = self
            .source_micro_frames
            .checked_sub(earlier.source_micro_frames)?;
        (wall > 0).then(|| micro as f64 / 1e6 / wall as f64)
    }
}

/// Everything an [`Engine`] is wired to.
pub struct EngineConfig {
    /// The engine's own sample rate (stereo `f32`): the rate of the output stream in shared mode,
    /// where tracks of another rate are resampled to it on the decoder thread, and the rate the
    /// stream falls back to in exclusive mode (below). Everything the engine reports or accepts
    /// in frames (seek targets, start frames, positions) is counted at this rate, whatever rate
    /// the output stream runs at.
    ///
    /// In exclusive mode (`AudioOutput::wants_exclusive`) the output is reopened at the rate of
    /// each file instead of resampling it, and at this rate when the device refuses that: see
    /// [`Engine::poll`].
    pub sample_rate: u32,
    /// Output device id, `None` for the system default.
    pub device: Option<String>,
    /// Where track bytes come from.
    pub opener: Arc<dyn SourceOpener>,
    /// Where events go.
    pub sink: Arc<dyn EventSink>,
    /// Wall-clock source stamped onto events. Audio positions never use it.
    pub clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineConfig")
            .field("sample_rate", &self.sample_rate)
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

/// A command that has been sent but whose effect the decoder has not published yet.
#[derive(Debug, Clone, Copy)]
struct Pending {
    gen: u64,
    item: Option<QueueItemId>,
    frame: u64,
    /// A track is expected to start (false for `stop`).
    expect_audio: bool,
}

/// The playback engine. Owns the decoder thread and the output stream; every method is
/// non-blocking and none of them runs on the render callback.
///
/// Drive it from one thread (the app's playback controller): issue commands, and call
/// [`poll`](Self::poll) about every 100 ms. The core has no timer of its own: the host that wires
/// events to signals owns the cadence, and without `poll` no track-change, end-of-queue or
/// position-mark events are produced.
///
/// # Shutdown
/// Dropping the engine only signals the decoder thread and closes the stream; it does not wait,
/// so a stuck read can never hang application exit. [`shutdown`](Self::shutdown) signals and joins
/// the thread, for tests and orderly teardown.
pub struct Engine {
    sh: Arc<Shared>,
    tx: SyncSender<Cmd>,
    thread: Option<JoinHandle<()>>,
    output: Box<dyn AudioOutput>,
    stream: Option<Box<dyn OutputStream>>,
    sink: Arc<dyn EventSink>,
    clock: Arc<dyn Clock>,
    /// The timeline rate (`EngineConfig::sample_rate`).
    sample_rate: u32,
    /// The rate the output stream runs at now: `sample_rate`, or a file's rate in exclusive mode.
    stream_rate: u32,
    /// Rates the device refused in exclusive mode since the device was last chosen.
    refused: Vec<u32>,
    /// The last reopen request answered (or given up on); a request is answered once.
    reopen_done: u32,
    device: Option<String>,
    gen: u64,
    /// The generation of the last command that reached the decoder's queue.
    sent_gen: u64,
    /// The `Resume` for a stream that is open already, which the decoder's full command queue
    /// has not taken yet, with the stream's info (reported once it is delivered). Retried by
    /// every poll; commands are refused as busy meanwhile, so none can overtake it.
    unsent_resume: Option<(Cmd, StreamInfo)>,
    /// Queue edits sent so far (the last [`EditId`]).
    edits: u64,
    pending: Option<Pending>,
    /// Mark of the track the control side currently reports as playing.
    cursor: Option<Mark>,
    /// Highest mark sequence already turned into events (marks never repeat or reorder).
    seen_seq: Option<u64>,
    /// Track (and start frame) a command already announced with `TrackStarted`; adopting its first
    /// mark emits nothing, and if another item starts instead, the announced one is closed as failed.
    announced: Option<(QueueItemId, u64)>,
    /// Output frame at which the current stream began, and the number of 100 ms marks already
    /// reported since then.
    grid_base: u64,
    last_grid: u64,
    /// Output frame (the shared `consumed` count) up to which the listener has heard the old
    /// stream: set when a stream is replaced at a track boundary, whose tail was played out first.
    /// The device latency of the new stream must not put the position back before it.
    heard_floor: u64,
    reported_underruns: u64,
    output_lost_reported: bool,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("sample_rate", &self.sample_rate)
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// Start the decoder thread and open the output stream. The stream's [`StreamInfo`](crate::StreamInfo) is
    /// reported at once as [`EventKind::OutputOpened`].
    ///
    /// # Errors
    /// [`PlaybackError::InvalidArgument`] for a zero sample rate; whatever the output reports when
    /// it cannot open (the decoder thread is stopped again).
    pub fn start(
        config: EngineConfig,
        mut output: Box<dyn AudioOutput>,
    ) -> Result<Self, PlaybackError> {
        let EngineConfig {
            sample_rate,
            device,
            opener,
            sink,
            clock,
        } = config;
        if sample_rate == 0 {
            return Err(PlaybackError::InvalidArgument("sample_rate must be > 0"));
        }
        let sh = Shared::new(1.0);
        sh.fade_frames.store(fade_frames(sample_rate), Relaxed);
        let (producer, consumer) = ring(sample_rate);
        let (tx, rx) = sync_channel(COMMAND_QUEUE);
        let decoder = Decoder {
            sh: Arc::clone(&sh),
            rx,
            producer,
            opener,
            sink: Arc::clone(&sink),
            clock: Arc::clone(&clock),
            sample_rate,
        };
        let thread = std::thread::Builder::new()
            .name("tinwave-decoder".into())
            .spawn(move || decoder.run())
            .map_err(|e| PlaybackError::Output(format!("cannot start decoder thread: {e}")))?;
        let request = OpenRequest {
            device: device.as_deref(),
            sample_rate,
            exclusive_only: false,
        };
        let stream = match output.open(&request, Renderer::new(consumer, Arc::clone(&sh))) {
            Ok(stream) => stream,
            Err(error) => {
                sh.shutdown.store(true, Release);
                let _ = tx.try_send(Cmd::Shutdown);
                let _ = thread.join();
                return Err(error);
            }
        };
        let info = stream.info();
        sh.follow.store(
            output.wants_exclusive() && info.mode == StreamMode::Exclusive,
            Release,
        );
        let engine = Self {
            sh,
            tx,
            thread: Some(thread),
            output,
            stream: Some(stream),
            sink,
            clock,
            sample_rate,
            stream_rate: sample_rate,
            refused: Vec::new(),
            reopen_done: 0,
            device,
            gen: 0,
            sent_gen: 0,
            unsent_resume: None,
            edits: 0,
            pending: None,
            cursor: None,
            seen_seq: None,
            announced: None,
            grid_base: 0,
            last_grid: 0,
            heard_floor: 0,
            reported_underruns: 0,
            output_lost_reported: false,
        };
        engine.emit(EventKind::OutputOpened {
            device: engine.device.clone(),
            info,
        });
        Ok(engine)
    }

    /// Replace the queue and start playing `items[index]` at `start_frame`. Skip, previous and
    /// "play this" are all `load`. Keeps the paused state.
    ///
    /// # Errors
    /// [`PlaybackError::InvalidArgument`] for an empty queue, an out-of-range index or duplicate
    /// item ids; [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`] when the decoder
    /// thread cannot take the command.
    pub fn load(
        &mut self,
        items: Vec<Track>,
        index: usize,
        start_frame: u64,
    ) -> Result<(), PlaybackError> {
        if items.is_empty() || index >= items.len() {
            return Err(PlaybackError::InvalidArgument("index is outside the queue"));
        }
        check_unique(&items)?;
        self.poll();
        let leaving = self.current_or_announced();
        let queue: Vec<Arc<Track>> = items.into_iter().map(Arc::new).collect();
        let target = queue[index].id;
        let gen = self.next_gen();
        self.send(Cmd::Load {
            queue,
            index,
            frame: start_frame,
            gen,
        })?;
        if let Some(left) = leaving {
            self.emit(EventKind::TrackLeft {
                item: left.item,
                frame: left.frame,
                cause: LeaveCause::Replaced,
            });
        }
        self.emit(EventKind::TrackStarted {
            item: target,
            frame: start_frame,
            cause: StartCause::Requested,
        });
        self.command_sent(gen, Some(target), start_frame, true);
        Ok(())
    }

    /// Add tracks after the last queue entry; if the queue had already ended, playback continues
    /// with them.
    ///
    /// # Errors
    /// As [`load`](Self::load).
    pub fn append(&mut self, items: Vec<Track>) -> Result<(), PlaybackError> {
        check_unique(&items)?;
        self.send(Cmd::Append(items.into_iter().map(Arc::new).collect()))
    }

    /// Replace everything after `head` with `items`, without touching the track that is playing:
    /// play next, reorder and remove are all this call with a different list. Nothing is flushed
    /// and nothing is reloaded, so the current track keeps playing and the hand-off into the first
    /// of `items` is as gapless as into any other entry.
    ///
    /// `head` is the item the caller saw as current, and the edit applies only if it still is.
    /// The decoder reads up to a second ahead, so the item after the current one is already
    /// started during the last second of the current track; an edit made then is refused.
    ///
    /// - A `head` that is not the current item fails this call with [`PlaybackError::StaleQueue`].
    /// - A `head` that is current here but not on the decoder thread (the race above) is refused
    ///   there: the call succeeds and [`EventKind::QueueEditRejected`] follows with the same
    ///   [`EditId`]. The edit is applied when [`EventKind::QueueEdited`] follows instead.
    ///
    /// Either way, a refused edit changes nothing, so a caller that records the edit (a signal)
    /// does so on `QueueEdited`.
    ///
    /// In exclusive mode the edit may change the sample rate the output has to be reopened at for
    /// the next track. Until the engine has started reopening it, the request for the old
    /// successor's rate is withdrawn and the new successor decides: a track at the rate of the
    /// playing one continues gapless, any other rate asks for its own stream. The listener hears
    /// the same as without the edit.
    ///
    /// On the last item of the queue, `items` continue the queue like [`append`](Self::append)
    /// does after the end, with the same caveat: an edit that reaches the decoder only after the
    /// output has played the last frame of the head finds the queue already ended (`QueueEnded`),
    /// and what plays then is announced when it is heard.
    ///
    /// # Errors
    /// [`PlaybackError::NothingLoaded`] when no track is current; [`PlaybackError::StaleQueue`] as
    /// above; [`PlaybackError::InvalidArgument`] for duplicate ids among `items` or an item with
    /// the id of `head`; otherwise as [`load`](Self::load).
    pub fn replace_after(
        &mut self,
        head: QueueItemId,
        items: Vec<Track>,
    ) -> Result<EditId, PlaybackError> {
        check_unique(&items)?;
        if items.iter().any(|t| t.id == head) {
            return Err(PlaybackError::InvalidArgument(
                "queue item ids must be unique",
            ));
        }
        self.poll();
        let current = self
            .current_or_announced()
            .ok_or(PlaybackError::NothingLoaded)?;
        if current.item != head {
            return Err(PlaybackError::StaleQueue {
                expected: head,
                current: Some(current.item),
            });
        }
        let edit = EditId(self.edits + 1);
        self.send(Cmd::EditQueue {
            edit,
            head,
            items: items.into_iter().map(Arc::new).collect(),
        })?;
        self.edits += 1;
        Ok(edit)
    }

    /// Stop playback and clear the queue.
    ///
    /// # Errors
    /// [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn stop(&mut self) -> Result<(), PlaybackError> {
        self.poll();
        let leaving = self.current_or_announced();
        let gen = self.next_gen();
        self.send(Cmd::Load {
            queue: Vec::new(),
            index: 0,
            frame: 0,
            gen,
        })?;
        if let Some(left) = leaving {
            self.emit(EventKind::TrackLeft {
                item: left.item,
                frame: left.frame,
                cause: LeaveCause::Stopped,
            });
        }
        self.command_sent(gen, None, 0, false);
        Ok(())
    }

    /// Seek the current track to `frame` of its (trimmed) timeline. Lands sample-exact.
    ///
    /// There must be a current track. Once the queue has ended (`QueueEnded`) or after `stop`
    /// there is none, and the seek is **rejected** with [`PlaybackError::NothingLoaded`] and emits
    /// nothing: restarting a finished track is a [`load`](Self::load), which announces it with its
    /// own `TrackStarted{Requested}` so that every start has exactly one matching leave.
    ///
    /// # Errors
    /// [`PlaybackError::NothingLoaded`], or as [`load`](Self::load). A target behind the end of the
    /// track is reported later as a `SeekFailed` event and the queue moves on.
    pub fn seek(&mut self, frame: u64) -> Result<(), PlaybackError> {
        self.poll();
        if self.cursor.is_none() && self.announced.is_none() {
            return Err(PlaybackError::NothingLoaded);
        }
        let from = self.position().ok_or(PlaybackError::NothingLoaded)?;
        let gen = self.next_gen();
        self.send(Cmd::Seek {
            item: from.item,
            frame,
            gen,
        })?;
        self.emit(EventKind::Seeked {
            item: from.item,
            from_frame: from.frame,
            to_frame: frame,
        });
        self.command_sent(gen, Some(from.item), frame, true);
        Ok(())
    }

    /// Pause with a one-block fade. No-op when already paused.
    pub fn pause(&mut self) {
        if !self.sh.paused.swap(true, Relaxed) {
            self.emit(EventKind::Paused {
                position: self.position(),
            });
        }
    }

    /// Resume with a one-block fade. No-op when not paused.
    pub fn resume(&mut self) {
        if self.sh.paused.swap(false, Relaxed) {
            self.emit(EventKind::Resumed {
                position: self.position(),
            });
        }
    }

    /// Whether playback is paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.sh.paused.load(Relaxed)
    }

    /// Set the software volume target (`volume_linear`, 0.0 to 1.0). The renderer ramps to it
    /// across one block, so there is no zipper noise.
    ///
    /// # Errors
    /// [`PlaybackError::InvalidArgument`] for NaN or a value outside 0.0 to 1.0.
    pub fn set_volume(&mut self, volume_linear: f32) -> Result<(), PlaybackError> {
        if !(0.0..=1.0).contains(&volume_linear) {
            return Err(PlaybackError::InvalidArgument(
                "volume_linear must be within 0.0 to 1.0",
            ));
        }
        let from = self.sh.volume();
        if from.to_bits() == volume_linear.to_bits() {
            return Ok(());
        }
        self.sh.volume.store(volume_linear.to_bits(), Relaxed);
        self.emit(EventKind::VolumeChanged {
            from,
            to: volume_linear,
            position: self.position(),
        });
        Ok(())
    }

    /// The software volume target.
    #[must_use]
    pub fn volume(&self) -> f32 {
        self.sh.volume()
    }

    /// Set the playback rate: source frames per output frame, pitch preserved. 1.0 is normal speed
    /// and bypasses the time stretcher, so playback is bit-exact; the range is
    /// [`MIN_RATE`] to [`MAX_RATE`] (0.75 to 1.5). The rate holds across tracks and loads.
    ///
    /// Returns at once. The decoder thread applies the change by restarting the stream at the
    /// audible position: the renderer fades out at most 5 ms, the decoder reopens the track there
    /// and the new stream fades in: a short dip whose length on a real device is not yet measured
    /// (ADR 0006, implementation note 7). A change that follows another within 125 ms waits for the interval,
    /// and the value current then is applied, so a drag costs a few restarts and ends on the
    /// final value. Setting the rate the engine already has does nothing.
    ///
    /// Every accepted call is reported as one [`EventKind::RateChanged`].
    ///
    /// # Errors
    /// [`PlaybackError::InvalidArgument`] for NaN or a rate outside the range;
    /// [`PlaybackError::EngineStopped`] when the decoder thread has exited.
    pub fn set_rate(&mut self, rate: f64) -> Result<(), PlaybackError> {
        if !(MIN_RATE..=MAX_RATE).contains(&rate) {
            return Err(PlaybackError::InvalidArgument(
                "rate must be within 0.75 to 1.5",
            ));
        }
        let from = self.sh.rate_target();
        if from.to_bits() == rate.to_bits() {
            return Ok(());
        }
        // The target is stored first: the wake-up only tells the decoder to look at it.
        self.sh.rate_target.store(rate.to_bits(), Release);
        if let Err(TrySendError::Disconnected(_)) = self.tx.try_send(Cmd::RateTarget) {
            self.sh.rate_target.store(from.to_bits(), Release);
            return Err(PlaybackError::EngineStopped);
        }
        self.emit(EventKind::RateChanged {
            from,
            to: rate,
            position: self.position(),
        });
        Ok(())
    }

    /// The playback rate target (1.0 is normal speed). The audio follows within a few tens of
    /// milliseconds: see [`rate_stats`](Self::rate_stats) for the rate being played.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.sh.rate_target()
    }

    /// The renderer's account of what it has played and at which rate.
    #[must_use]
    pub fn rate_stats(&self) -> RateStats {
        RateStats {
            wall_frames: self.sh.consumed.load(Acquire),
            source_micro_frames: self.sh.src_micro.load(Relaxed),
            effective_rate: f64::from_bits(self.sh.eff_rate.load(Relaxed)),
            applied_rate: self.sh.stream_rate(),
            flushes: self.sh.rate_flushes.load(Acquire),
        }
    }

    /// Where playback is, from frames the output has taken minus the output latency. Resolution is
    /// one audio callback. After a command this reports the command's target until the decoder has
    /// processed it.
    ///
    /// While exclusive mode waits to replace the stream at a track boundary, the last frames of
    /// the old track are still in the device buffer: the position goes on through them to the end
    /// of the track, and stays there (never back) until the new stream is heard.
    ///
    /// The position always names a track that has a `TrackStarted`:
    /// - While a track is current, it is that track's: never a later track the decoder has just
    ///   opened but [`poll`](Self::poll) has not announced yet, and it stops at the end of the
    ///   current track until it does.
    /// - Right after a command, before the first frames of its track are heard, it is the
    ///   command's target.
    /// - With **no current track** it is `None`: nothing loaded, after `stop`, and after the queue
    ///   ended, even when a track appended to the ended queue is already decoding (it is announced
    ///   when it is heard). So `Paused`, `Resumed`, `VolumeChanged` and a command built on the
    ///   position never carry an item without a `TrackStarted`.
    #[must_use]
    pub fn position(&self) -> Option<Position> {
        if let Some(p) = self.unapplied() {
            return p.item.map(|item| self.position_of(item, p.frame));
        }
        if self.cursor.is_none() {
            // Not adopted yet: only what a command announced, never a mark nobody has announced.
            return self
                .announced
                .map(|(item, frame)| self.position_of(item, frame));
        }
        let view = self.announced(self.view());
        let (idx, frame) = resolve(&view.marks, view.shown)?;
        Some(self.position_of(view.marks[idx].item.id, frame))
    }

    /// `view` as far as the control side has announced it: the marks it has not turned into
    /// events yet are left out, and the position stops at the first of them.
    fn announced(&self, mut view: View) -> View {
        if let Some(seen) = self.seen_seq {
            if let Some(next) = view.marks.iter().find(|m| m.seq > seen) {
                view.shown = view.shown.min(next.start_frame);
                view.marks.retain(|m| m.seq <= seen);
            }
        }
        view
    }

    /// True from a `load`, `seek`, `stop` or `switch_device` call until the decoder thread has
    /// carried it out and the new stream is the one being rendered. `position` reports the
    /// command's target meanwhile. In exclusive mode a command that starts a track of another
    /// sample rate is carried out once [`poll`](Self::poll) has reopened the output at that rate.
    #[must_use]
    pub fn command_pending(&self) -> bool {
        self.unapplied().is_some()
    }

    /// Move to another output device (`None` is the system default) and resume at the exact frame
    /// that was playing. Frames already in the old device's buffer are not replayed.
    ///
    /// On error the engine has no open stream (the old one is already closed); call again. The
    /// exception is [`PlaybackError::Busy`] while a reopened stream still waits for the decoder to
    /// take its ring: then nothing is touched and the current stream keeps playing.
    ///
    /// # Errors
    /// What the output reports; [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn switch_device(&mut self, device: Option<String>) -> Result<(), PlaybackError> {
        self.poll();
        if self.unsent_resume.is_some() {
            // The stash belongs to the stream that is open now: closing it would leave the
            // decoder resuming into a dead ring and the next poll reporting a reopen of nothing.
            return Err(PlaybackError::Busy);
        }
        // A track that is decoded but not heard yet (appended after the queue ended, within one
        // output latency): resumed from its start, and announced here because nothing else will.
        let mut unheard_start: Option<(QueueItemId, u64)> = None;
        let resume = match self.unapplied() {
            Some(p) => p.item.map(|item| (item, p.frame)),
            None if self.cursor.is_none() && self.announced.is_none() => {
                // No current track (never loaded, stopped, or the queue ended). Normally there is
                // nothing to resume and the decoder carries on with whatever has been appended,
                // from its start. But an appended track the decoder already opened has a mark and
                // the decoder is past it, so it must be named or it would be dropped.
                let view = self.view();
                unheard_start = view
                    .marks
                    .iter()
                    .find(|m| self.seen_seq.is_none_or(|seen| m.seq > seen))
                    .map(|m| (m.item.id, m.offset));
                unheard_start
            }
            None => self.position().map(|p| (p.item, p.frame)),
        };
        // The old callback must be gone before the decoder re-bases on `consumed`.
        self.stream = None;
        // A new device: what the old one refused says nothing about it.
        self.refused.clear();
        // Exclusive mode keeps the rate of what is playing; the track then follows by itself.
        let opened = self.open_stream(device.as_deref(), self.stream_rate);
        let gen = self.next_gen();
        let resume_item = resume.map(|r| r.0);
        let resume_frame = resume.map_or(0, |r| r.1);
        let (producer, rate, result) = match opened {
            Ok(o) => (o.producer, o.rate, Ok((o.stream, o.info))),
            // The decoder restarts regardless (below), into a ring nobody plays.
            Err(error) => (ring(self.stream_rate).0, self.stream_rate, Err(error)),
        };
        if let Err(error) = self.send(Cmd::Reconfigure {
            producer,
            rate,
            item: resume_item,
            frame: resume_frame,
            gen,
        }) {
            // Nothing was sent, but the old stream is already closed (a new one that opened is
            // dropped with `result`).
            self.output_lost_reported = true;
            self.emit(EventKind::OutputLost);
            return Err(error);
        }
        // From here the decoder WILL restart the stream, whether or not the new device opens, so
        // the command is recorded either way: its first mark is adopted silently instead of being
        // announced a second time.
        if let Some((item, frame)) = unheard_start {
            self.emit(EventKind::TrackStarted {
                item,
                frame,
                cause: StartCause::Advance,
            });
        }
        self.command_sent(gen, resume_item, resume_frame, true);
        match result {
            Ok((stream, info)) => {
                self.stream = Some(stream);
                self.device = device.clone();
                self.output_lost_reported = false;
                self.emit(EventKind::OutputReopened { device, info });
                Ok(())
            }
            Err(error) => {
                self.output_lost_reported = true;
                self.emit(EventKind::OutputLost);
                Err(error)
            }
        }
    }

    /// Turn what the audio callback has done into events: track changes, the end of the queue,
    /// 10 Hz position marks, underruns and a lost output. Call about every 100 ms; the position
    /// marks follow heard audio, not the call rate, so a late call produces one mark, not a burst.
    pub fn poll(&mut self) {
        self.report_underruns_and_output();
        self.service_reopen();
        if self.unapplied().is_some() {
            return;
        }
        if let Some(done) = self.pending.take() {
            self.pending_applied(done);
        }
        let view = self.view();

        let ended = view.ended;
        let heard = if ended { view.consumed } else { view.played };
        let reached = view
            .marks
            .iter()
            .rposition(|m| m.start_frame <= heard)
            .map_or(0, |i| i + 1);
        for mark in &view.marks[..reached] {
            if self.seen_seq.is_some_and(|seen| mark.seq <= seen) {
                continue;
            }
            match (&self.cursor, mark.kind) {
                // The stream restarted at a new rate and carries on with the same track: the
                // listener's view does not change.
                (Some(prev), MarkKind::Rate) if prev.item.id == mark.item.id => {}
                (Some(prev), MarkKind::Continuation | MarkKind::Rate) => {
                    let left_at = prev.source_frame_at(mark.start_frame);
                    let (left, started) = (prev.item.id, mark.item.id);
                    let cause = if mark.prev_failed {
                        LeaveCause::Failed
                    } else {
                        LeaveCause::Finished
                    };
                    self.emit(EventKind::TrackLeft {
                        item: left,
                        frame: left_at,
                        cause,
                    });
                    self.emit(EventKind::TrackStarted {
                        item: started,
                        frame: mark.offset,
                        cause: StartCause::Advance,
                    });
                    if prev.out_rate != mark.out_rate {
                        // The output was reopened at another rate: heard frames are counted in
                        // it from here, so the 10 Hz grid starts again.
                        self.grid_base = mark.start_frame;
                        self.last_grid = 0;
                    }
                }
                _ => {
                    // Adopt the stream a command started; an item the command did not name means
                    // the named one failed to open and a later entry took over.
                    let announced = self.announced.take();
                    if announced.map(|a| a.0) != Some(mark.item.id) {
                        if let Some((item, frame)) = announced {
                            self.emit(EventKind::TrackLeft {
                                item,
                                frame,
                                cause: LeaveCause::Failed,
                            });
                        }
                        self.emit(EventKind::TrackStarted {
                            item: mark.item.id,
                            frame: mark.offset,
                            cause: StartCause::Advance,
                        });
                    }
                    // Marks count heard audio from where this stream began.
                    self.grid_base = mark.start_frame;
                    self.last_grid = 0;
                }
            }
            self.seen_seq = Some(mark.seq);
            self.cursor = Some(mark.clone());
        }

        if ended {
            if let Some(last) = self.cursor.take() {
                let frame = last.source_frame_at(view.eos_total);
                let cause = if view.eos_failed {
                    LeaveCause::Failed
                } else {
                    LeaveCause::Finished
                };
                self.emit(EventKind::TrackLeft {
                    item: last.item.id,
                    frame,
                    cause,
                });
                self.emit(EventKind::QueueEnded);
            }
        } else if self.cursor.is_some() && !self.sh.paused.load(Relaxed) {
            let out_rate = self
                .cursor
                .as_ref()
                .map_or(self.sample_rate, |c| c.out_rate);
            let rate_interval = u64::from(out_rate / POSITION_MARKS_PER_SECOND).max(1);
            // What the listener hears of the current track, including the tail of an old stream
            // that is playing out before the output is reopened (the marks of events above still
            // wait for the next track to be heard).
            let view = self.announced(view);
            let grid = view.shown.saturating_sub(self.grid_base) / rate_interval;
            if grid > self.last_grid {
                self.last_grid = grid;
                if let Some((idx, frame)) = resolve(&view.marks, view.shown) {
                    let position = self.position_of(view.marks[idx].item.id, frame);
                    self.emit(EventKind::PositionMark { position });
                }
            }
        }

        if let Some(seen) = self.seen_seq {
            self.sh.marks().retain(|m| m.seq >= seen);
        }
    }

    /// Whether the decoder is waiting for [`poll`](Self::poll) to reopen the output stream at the
    /// sample rate of its next track (exclusive mode), and a poll can make progress with it. A
    /// host that polls slowly can poll faster while this is true: playback of that track starts
    /// when the poll has reopened the stream.
    ///
    /// At a track boundary the request waits for the old stream to play out. While paused it
    /// does not, so a pause in the last second before a rate change is not "pending": nothing a
    /// poll does would change until playback resumes.
    #[must_use]
    pub fn reopen_pending(&self) -> bool {
        if self.unsent_resume.is_some() {
            return self.stream.is_some();
        }
        let (id, rate) = self.sh.reopen_request();
        let waits_for_playback = self.sh.reopen_drain.load(Acquire) && self.is_paused();
        rate != 0 && id != self.reopen_done && self.stream.is_some() && !waits_for_playback
    }

    /// Whether the host should poll sooner than its usual interval: a command (load, seek, stop,
    /// device switch) has yet to be carried out, or the output has to be reopened, and a poll
    /// would move either along. False once the output is lost (nothing a poll does helps until
    /// the host switches device), so a stuck command does not keep the host awake.
    #[must_use]
    pub fn poll_soon(&self) -> bool {
        self.stream.is_some()
            && !self.output_lost_reported
            && (self.command_pending() || self.reopen_pending())
    }

    /// Stop the decoder thread and wait for it. (Dropping the engine stops it too but does not
    /// wait, so a stuck read can never hang the application.)
    pub fn shutdown(mut self) {
        self.stream = None;
        self.request_shutdown();
        if let Some(thread) = self.thread.take() {
            // A panic in the decoder thread has already been reported by the panic hook.
            let _ = thread.join();
        }
    }

    // --- internals -------------------------------------------------------------------------

    /// Answer the decoder's request for an output stream at another sample rate (exclusive mode,
    /// ADR 0005 decision 5), when the old stream has nothing left to play.
    ///
    /// The decoder asks when the next track's rate is not the stream's. At a track boundary the
    /// request waits until the old stream has played its last frame and its device buffer has
    /// emptied, so no audio is cut; after a load or seek there is nothing left to wait for. The
    /// old stream is then closed, a new one opened at the track's rate (or at the engine's own
    /// rate when the device refuses that, see [`open_stream`](Self::open_stream)), and the decoder
    /// carries on in it: the marks, the positions and the track events continue as at any other
    /// boundary. The listener hears a gap of the old device's buffer, the time the device takes to
    /// open, and up to one poll interval.
    ///
    /// The engine takes the request (`Shared::take_reopen`) before it closes the old stream; the
    /// decoder withdraws a request when a queue edit changed the track it was for, and whichever
    /// takes it first wins, so a `Resume` is only ever sent for a request the decoder still waits
    /// on.
    ///
    /// Events: one [`EventKind::OutputReopened`] with the new stream's [`StreamInfo`] (a refused
    /// rate is in its `fallback`); [`EventKind::OutputLost`] when no stream could be opened.
    fn service_reopen(&mut self) {
        if let Some((cmd, info)) = self.unsent_resume.take() {
            self.deliver_resume(cmd, info);
            return;
        }
        // The generation first: the decoder stores it after withdrawing a request, so a request
        // read after a matching generation is the decoder's answer to the latest command.
        if self.sh.reopen_gen.load(Acquire) != self.sent_gen {
            return;
        }
        let (id, rate) = self.sh.reopen_request();
        if rate == 0 || id == self.reopen_done || self.stream.is_none() {
            return;
        }
        let (total, drain) = (
            self.sh.reopen_total.load(Acquire),
            self.sh.reopen_drain.load(Acquire),
        );
        if self.sh.consumed.load(Acquire) < total {
            return;
        }
        let emptied = self.sh.idle_frames.load(Relaxed) >= self.sh.latency_frames.load(Relaxed);
        if drain && !emptied {
            return;
        }
        // The request is the engine's before anything is closed: the decoder withdraws one when a
        // queue edit changed the track it was parked for, and it can do that until now. Losing
        // means it did (or a newer request is there): nothing is reopened, the next poll looks again.
        if !self.sh.take_reopen(id, rate) {
            return;
        }
        // Answered once, whatever the outcome: a device that cannot open is not retried every poll.
        self.reopen_done = id;
        if drain {
            // The old stream has played to its last frame and its device buffer has emptied: all of
            // it has been heard, whatever the latency of the stream that replaces it.
            self.heard_floor = total;
        }
        self.stream = None;
        let device = self.device.clone();
        match self.open_stream(device.as_deref(), rate) {
            Ok(opened) => {
                let cmd = Cmd::Resume {
                    producer: opened.producer,
                    rate: opened.rate,
                    id,
                };
                self.stream = Some(opened.stream);
                self.output_lost_reported = false;
                self.deliver_resume(cmd, opened.info);
            }
            Err(error) => {
                tracing::warn!(%error, rate, "the output could not be reopened for the next track");
                self.output_lost_reported = true;
                self.emit(EventKind::OutputLost);
            }
        }
    }

    /// Hand the decoder the ring of the stream that was just opened, and report the reopen. A
    /// full command queue is transient (the parked decoder is draining it): the command is kept
    /// and the next poll tries again, with the stream already running on its empty ring.
    fn deliver_resume(&mut self, cmd: Cmd, info: StreamInfo) {
        match self.tx.try_send(cmd) {
            Ok(()) => {
                let device = self.device.clone();
                self.emit(EventKind::OutputReopened { device, info });
            }
            Err(TrySendError::Full(cmd)) => self.unsent_resume = Some((cmd, info)),
            Err(TrySendError::Disconnected(_)) => {
                self.stream = None;
                self.output_lost_reported = true;
                self.emit(EventKind::OutputLost);
            }
        }
    }

    /// Open an output stream, and publish what the decoder needs to know about it. `rate` is
    /// what the next track needs.
    ///
    /// In shared mode the stream always runs at the engine's rate. In exclusive mode
    /// (`AudioOutput::wants_exclusive`) it runs at `rate`, so the file plays bit for bit; a device
    /// that does not open `rate` exclusively is a refusal: the rate is remembered (the decoder
    /// resamples tracks of that rate from then on, once), and the stream is opened again at the
    /// engine's rate, exclusive if the device allows that and shared otherwise. The reason is in
    /// the returned info's `fallback`. The attempt at `rate` is exclusive-only
    /// ([`OpenRequest::exclusive_only`]), so a refusing output answers with an error and no
    /// shared stream is built just to be dropped: a refusal costs one failed open.
    fn open_stream(
        &mut self,
        device: Option<&str>,
        rate: u32,
    ) -> Result<OpenedStream, PlaybackError> {
        let exclusive = self.output.wants_exclusive();
        let mut rate = if exclusive { rate } else { self.sample_rate };
        let mut refusal: Option<String> = None;
        loop {
            // Before the renderer exists: it reads this when a rate change flushes its ring.
            self.sh.fade_frames.store(fade_frames(rate), Relaxed);
            let (producer, consumer) = ring(rate);
            let request = OpenRequest {
                device,
                sample_rate: rate,
                exclusive_only: exclusive && rate != self.sample_rate,
            };
            let renderer = Renderer::new(consumer, Arc::clone(&self.sh));
            let opened = match self.output.open(&request, renderer) {
                Ok(stream) => {
                    let mut info = stream.info();
                    if request.exclusive_only && info.mode != StreamMode::Exclusive {
                        // An output that ignores `exclusive_only` hands back a shared stream.
                        let why = info
                            .fallback
                            .take()
                            .unwrap_or_else(|| "the device did not open it exclusively".into());
                        // Closed before the next open: an exclusive endpoint has one client.
                        drop(stream);
                        Err(why)
                    } else {
                        Ok((stream, info))
                    }
                }
                // Only the device saying no to the rate is a refusal; any other failure ends the
                // open, as it does for a rate that is not exclusive-only.
                Err(error @ PlaybackError::ExclusiveRefused(_)) if request.exclusive_only => {
                    Err(error.to_string())
                }
                Err(error) => return Err(error),
            };
            let (stream, mut info) = match opened {
                Ok(opened) => opened,
                Err(why) => {
                    if !self.refused.contains(&rate) {
                        self.refused.push(rate);
                    }
                    refusal = Some(format!(
                        "{rate} Hz refused ({why}); using {} Hz and resampling",
                        self.sample_rate
                    ));
                    rate = self.sample_rate;
                    continue;
                }
            };
            if let Some(note) = refusal {
                info.fallback = Some(match info.fallback.take() {
                    Some(other) => format!("{note}; {other}"),
                    None => note,
                });
            }
            self.stream_rate = rate;
            self.sh.refused().clone_from(&self.refused);
            self.sh
                .follow
                .store(exclusive && info.mode == StreamMode::Exclusive, Release);
            return Ok(OpenedStream {
                stream,
                producer,
                rate,
                info,
            });
        }
    }

    fn request_shutdown(&self) {
        self.sh.shutdown.store(true, Release);
        // Only a wake-up: the flag is what matters, so a full queue is fine.
        let _ = self.tx.try_send(Cmd::Shutdown);
    }

    /// Where the track that this command replaces is: the playing one, or one a previous command
    /// announced but that has not been heard yet (so every `TrackStarted` gets a `TrackLeft`).
    fn current_or_announced(&self) -> Option<Position> {
        if self.cursor.is_some() || self.announced.is_some() {
            self.position()
        } else {
            None
        }
    }

    fn next_gen(&mut self) -> u64 {
        self.gen += 1;
        self.gen
    }

    fn send(&self, cmd: Cmd) -> Result<(), PlaybackError> {
        if self.unsent_resume.is_some() {
            // The decoder is waiting for a stream's ring: nothing may overtake it.
            return Err(PlaybackError::Busy);
        }
        self.tx.try_send(cmd).map_err(|e| match e {
            TrySendError::Full(_) => PlaybackError::Busy,
            TrySendError::Disconnected(_) => PlaybackError::EngineStopped,
        })
    }

    fn command_sent(
        &mut self,
        gen: u64,
        item: Option<QueueItemId>,
        frame: u64,
        expect_audio: bool,
    ) {
        self.cursor = None;
        self.sent_gen = gen;
        self.announced = item.filter(|_| expect_audio).map(|i| (i, frame));
        self.pending = Some(Pending {
            gen,
            item,
            frame,
            expect_audio: expect_audio && item.is_some(),
        });
    }

    /// The pending command, if the decoder has not published its effect yet.
    fn unapplied(&self) -> Option<Pending> {
        self.pending
            .filter(|p| self.sh.applied_gen.load(Acquire) < p.gen)
    }

    /// A command's effect is now visible. If it produced no track at all, the queue is over.
    fn pending_applied(&mut self, done: Pending) {
        if done.expect_audio && self.sh.marks().is_empty() {
            if let Some((item, frame)) = self.announced.take() {
                self.emit(EventKind::TrackLeft {
                    item,
                    frame,
                    cause: LeaveCause::Failed,
                });
            }
            self.emit(EventKind::QueueEnded);
        }
    }

    fn view(&self) -> View {
        let consumed = self.sh.consumed.load(Acquire);
        let latency = self.sh.latency_frames.load(Relaxed);
        let eos = self.sh.eos.load(Acquire);
        let eos_total = self.sh.eos_total.load(Acquire);
        let eos_failed = self.sh.eos_failed.load(Acquire);
        let marks: Vec<Mark> = self.sh.marks().iter().cloned().collect();
        let ended = eos && consumed >= eos_total && !marks.is_empty();
        let played = if ended {
            consumed
        } else {
            consumed.saturating_sub(latency)
        };
        // The position the listener is at: `played`, plus what the device plays of its buffered
        // tail while the decoder waits for the stream to be replaced (no new frames are consumed
        // then, but the old ones are still being heard), and never before the end of a stream
        // that has been replaced (the new stream's latency starts counting there). Events keep
        // using `played`: the next track is heard when its own frames are.
        let (_, requested) = self.sh.reopen_request();
        let draining = requested != 0
            && self.sh.reopen_drain.load(Acquire)
            && consumed >= self.sh.reopen_total.load(Acquire);
        let credit = if draining {
            self.sh.idle_frames.load(Relaxed).min(latency)
        } else {
            0
        };
        let shown = if ended {
            consumed
        } else {
            (played + credit).min(consumed).max(self.heard_floor)
        };
        View {
            consumed,
            played,
            shown,
            eos_total,
            eos_failed,
            ended,
            marks,
        }
    }

    fn position_of(&self, item: QueueItemId, frame: u64) -> Position {
        Position {
            item,
            frame,
            position_ms: frame.saturating_mul(1000) / u64::from(self.sample_rate),
        }
    }

    fn report_underruns_and_output(&mut self) {
        let underruns = self.sh.underruns.load(Relaxed);
        if underruns > self.reported_underruns {
            let callbacks = underruns - self.reported_underruns;
            self.reported_underruns = underruns;
            self.emit(EventKind::Underrun { callbacks });
        }
        let lost = self.stream.as_ref().is_some_and(|s| s.is_lost());
        if lost && !self.output_lost_reported {
            self.output_lost_reported = true;
            self.emit(EventKind::OutputLost);
        }
    }

    fn emit(&self, kind: EventKind) {
        self.sink.emit(PlaybackEvent {
            at: self.clock.now(),
            kind,
        });
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stream = None;
        self.request_shutdown();
    }
}

/// An output stream the engine opened, and the ring that feeds it.
struct OpenedStream {
    stream: Box<dyn OutputStream>,
    producer: rtrb::Producer<f32>,
    rate: u32,
    info: StreamInfo,
}

#[derive(Debug)]
struct View {
    consumed: u64,
    /// Frames heard: consumed minus output latency, or all consumed once the stream has ended.
    played: u64,
    /// Frames heard as far as the position is concerned: `played`, completed while an old stream
    /// plays out its device buffer (see `Engine::view`).
    shown: u64,
    eos_total: u64,
    eos_failed: bool,
    ended: bool,
    marks: Vec<Mark>,
}

/// Which mark holds output frame `played`, and the frame inside its track. Before the first mark's
/// start (the old stream's tail still playing after a seek) it is the first mark's offset.
fn resolve(marks: &[Mark], played: u64) -> Option<(usize, u64)> {
    match marks.iter().rposition(|m| m.start_frame <= played) {
        Some(i) => Some((i, marks[i].source_frame_at(played))),
        None => marks.first().map(|m| (0, m.offset)),
    }
}

/// Frames of the fade a rate change plays: 5 ms at the stream rate.
fn fade_frames(sample_rate: u32) -> u32 {
    (sample_rate / (1000 / RATE_FADE_MS)).max(1)
}

fn ring(sample_rate: u32) -> (rtrb::Producer<f32>, rtrb::Consumer<f32>) {
    RingBuffer::new(sample_rate as usize * CHANNELS * RING_SECONDS)
}

fn check_unique(items: &[Track]) -> Result<(), PlaybackError> {
    let mut seen = HashSet::with_capacity(items.len());
    if items.iter().all(|t| seen.insert(t.id)) {
        Ok(())
    } else {
        Err(PlaybackError::InvalidArgument(
            "queue item ids must be unique",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tinwave_core::{ManualClock, UtcMillis};

    use super::*;
    use crate::output::{ManualHandle, ManualOutput};
    use crate::FsOpener;

    #[derive(Debug, Default)]
    struct Events(Mutex<Vec<EventKind>>);

    impl EventSink for Events {
        fn emit(&self, event: PlaybackEvent) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event.kind);
        }
    }

    impl Events {
        fn take(&self) -> Vec<EventKind> {
            std::mem::take(
                &mut *self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
        }
    }

    /// An engine in exclusive mode on a fake device, with nothing loaded: the decoder thread is
    /// idle, so the test can state a reopen request itself and watch what the engine does.
    fn exclusive_engine() -> (Engine, ManualHandle, Arc<Events>) {
        let (output, handle) = ManualOutput::new();
        handle.set_exclusive(true);
        let events = Arc::new(Events::default());
        let engine = Engine::start(
            EngineConfig {
                sample_rate: 48_000,
                device: None,
                opener: Arc::new(FsOpener),
                sink: events.clone(),
                clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
            },
            Box::new(output),
        )
        .expect("starts");
        assert_eq!(events.take().len(), 1, "OutputOpened");
        (engine, handle, events)
    }

    fn reopened(events: &[EventKind]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, EventKind::OutputReopened { .. }))
            .count()
    }

    #[test]
    fn a_reopen_request_is_served_only_for_the_latest_command_the_decoder_has_taken() {
        let (mut engine, handle, events) = exclusive_engine();
        // The decoder parked for generation 0; meanwhile the engine sent command 1 (still queued).
        engine.sh.park(1, 96_000, 0, false);
        engine.sent_gen = 1;
        assert!(engine.reopen_pending());
        engine.poll();
        assert_eq!(handle.opens().len(), 1, "a stale request is left alone");
        assert_eq!(reopened(&events.take()), 0);

        // The decoder has taken command 1 and parked again: now the engine answers.
        engine.sh.reopen_gen.store(1, Release);
        engine.poll();
        assert_eq!(handle.opens().last().map(|o| o.1), Some(96_000));
        assert_eq!(reopened(&events.take()), 1);
        assert_eq!(engine.sh.fade_frames.load(Relaxed), 480, "5 ms at 96 kHz");
        // ...once: the request is still published, but it has been answered.
        assert!(!engine.reopen_pending());
        engine.poll();
        assert_eq!(handle.opens().len(), 2);
        assert_eq!(reopened(&events.take()), 0);
    }

    #[test]
    fn the_engine_takes_the_request_it_answers_so_the_decoder_cannot_withdraw_it_anymore() {
        let (mut engine, handle, events) = exclusive_engine();
        engine.sh.park(1, 96_000, 0, false);
        engine.poll();
        assert_eq!(handle.opens().last().map(|o| o.1), Some(96_000));
        assert_eq!(reopened(&events.take()), 1);
        // A decoder that withdraws now (a queue edit that raced the reopen) loses: the stream it
        // would have withdrawn the request for is open, and its `Resume` is on the way.
        assert!(!engine.sh.take_reopen(1, 96_000));
    }

    #[test]
    fn a_request_the_decoder_withdrew_is_not_answered() {
        let (mut engine, handle, events) = exclusive_engine();
        engine.sh.park(1, 96_000, 0, false);
        assert!(
            engine.sh.take_reopen(1, 96_000),
            "the decoder got there first"
        );
        engine.poll();
        assert_eq!(handle.opens().len(), 1, "no stream was opened");
        assert_eq!(reopened(&events.take()), 0);
        assert!(!engine.reopen_pending());
        // The decoder parks again, for another rate: that request is answered.
        engine.sh.park(2, 44_100, 0, false);
        engine.poll();
        assert_eq!(handle.opens().last().map(|o| o.1), Some(44_100));
    }

    #[test]
    fn a_track_boundary_waits_for_the_last_frame_and_for_the_device_buffer_to_empty() {
        let (mut engine, handle, events) = exclusive_engine();
        engine.sh.latency_frames.store(480, Relaxed);
        engine.sh.park(1, 96_000, 1_000, true);

        // 999 of 1,000 frames played: not yet.
        engine.sh.consumed.store(999, Release);
        engine.poll();
        assert_eq!(handle.opens().len(), 1);
        // All played, the device still holds them (no silence has followed): not yet.
        engine.sh.consumed.store(1_000, Release);
        engine.sh.idle_frames.store(479, Relaxed);
        engine.poll();
        assert_eq!(
            handle.opens().len(),
            1,
            "the tail is still in the device buffer"
        );
        // The buffer has emptied.
        engine.sh.idle_frames.store(480, Relaxed);
        engine.poll();
        assert_eq!(handle.opens().len(), 2);
        assert_eq!(reopened(&events.take()), 1);
    }

    #[test]
    fn a_request_after_a_load_or_seek_does_not_wait_for_the_device_buffer() {
        let (mut engine, handle, _) = exclusive_engine();
        engine.sh.latency_frames.store(480, Relaxed);
        // Nothing to drain: the audio of the old stream was thrown away by the flush.
        engine.sh.consumed.store(5_000, Release);
        engine.sh.park(1, 44_100, 5_000, false);
        engine.poll();
        assert_eq!(handle.opens().last().map(|o| o.1), Some(44_100));
    }

    #[test]
    fn a_device_that_cannot_open_is_reported_lost_and_not_asked_again() {
        let (mut engine, handle, events) = exclusive_engine();
        engine.sh.park(1, 96_000, 0, false);
        handle.fail_next_open();
        engine.poll();
        assert_eq!(events.take(), vec![EventKind::OutputLost]);
        assert!(!handle.is_open());
        assert!(!engine.reopen_pending(), "answered, whatever the outcome");
        engine.poll();
        assert_eq!(handle.opens().len(), 1, "no retry on the next poll");
        assert!(events.take().is_empty());
    }

    #[test]
    fn shared_mode_never_follows_a_file_rate() {
        let (output, handle) = ManualOutput::new();
        let events = Arc::new(Events::default());
        let mut engine = Engine::start(
            EngineConfig {
                sample_rate: 48_000,
                device: None,
                opener: Arc::new(FsOpener),
                sink: events.clone(),
                clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
            },
            Box::new(output),
        )
        .expect("starts");
        assert!(!engine.sh.follow.load(Relaxed));
        engine.switch_device(Some("other".into())).expect("switch");
        assert_eq!(handle.opens().last().map(|o| o.1), Some(48_000));
        assert!(!engine.sh.follow.load(Relaxed));
    }

    #[test]
    fn refused_rates_are_remembered_per_device_and_published_to_the_decoder() {
        let (mut engine, handle, _) = exclusive_engine();
        handle.set_device_rates(&[48_000]);
        engine.sh.park(1, 96_000, 0, false);
        engine.poll();
        // 96 kHz refused, then the engine's rate taken (still exclusive).
        let opened: Vec<u32> = handle.opens().iter().map(|o| o.1).collect();
        assert_eq!(opened, [48_000, 96_000, 48_000]);
        assert_eq!(*engine.sh.refused(), vec![96_000]);
        assert!(engine.sh.follow.load(Relaxed));
        // A new device starts from a clean slate.
        handle.set_device_rates(&[48_000, 96_000]);
        engine.switch_device(Some("dac".into())).expect("switch");
        assert!(engine.sh.refused().is_empty());
    }

    #[test]
    fn a_boundary_waiting_for_playback_is_not_pending_while_paused() {
        let (mut engine, _handle, _) = exclusive_engine();
        engine.sh.park(1, 96_000, 1_000, true);
        assert!(engine.reopen_pending() && engine.poll_soon());
        // Paused in the last second before the boundary: no poll can move it along.
        engine.pause();
        assert!(
            !engine.reopen_pending(),
            "a drain wait does not advance while paused"
        );
        assert!(!engine.poll_soon());
        engine.resume();
        assert!(engine.reopen_pending() && engine.poll_soon());
        // A request with nothing to drain (after a load) is served by the next poll, paused or not.
        engine.sh.park(2, 44_100, 0, false);
        engine.pause();
        assert!(engine.reopen_pending());
    }

    #[test]
    fn a_lost_output_is_not_polled_quickly() {
        let (mut engine, handle, events) = exclusive_engine();
        // A command the decoder will never carry out (nothing renders): pending.
        engine.pending = Some(Pending {
            gen: 5,
            item: None,
            frame: 0,
            expect_audio: false,
        });
        assert!(engine.command_pending() && engine.poll_soon());
        handle.set_lost(true);
        engine.poll();
        assert_eq!(events.take(), vec![EventKind::OutputLost]);
        assert!(engine.command_pending(), "the command is still waiting...");
        assert!(!engine.poll_soon(), "...but polling quickly cannot help");

        // The same after a reopen that could not open a stream.
        let (mut engine, handle, _) = exclusive_engine();
        engine.sh.park(1, 96_000, 0, false);
        handle.fail_next_open();
        engine.poll();
        engine.sh.park(2, 44_100, 0, false);
        assert!(!engine.poll_soon());
    }

    #[test]
    fn a_full_command_queue_delays_the_resume_instead_of_losing_the_output() {
        let (mut engine, handle, events) = exclusive_engine();
        // A queue of one that nobody reads, already full.
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        engine.tx = tx;
        engine.tx.try_send(Cmd::RateTarget).expect("room for one");
        engine.sh.park(1, 96_000, 0, false);

        engine.poll();
        assert_eq!(handle.opens().last().map(|o| o.1), Some(96_000));
        assert!(handle.is_open(), "the new stream is running");
        assert!(
            events.take().is_empty(),
            "not reported until the decoder has its ring"
        );
        assert!(engine.reopen_pending(), "the next poll tries again");

        // Still full: still waiting, no second open.
        engine.poll();
        assert_eq!(handle.opens().len(), 2);
        assert!(events.take().is_empty());

        // Room again: the resume goes out and the reopen is reported, once.
        assert!(matches!(rx.try_recv(), Ok(Cmd::RateTarget)));
        // Nothing may overtake the resume, even though the queue has room now.
        assert_eq!(engine.append(Vec::new()).err(), Some(PlaybackError::Busy));
        engine.poll();
        assert!(matches!(
            rx.try_recv(),
            Ok(Cmd::Resume {
                id: 1,
                rate: 96_000,
                ..
            })
        ));
        assert_eq!(reopened(&events.take()), 1);
        assert!(!engine.reopen_pending());
        engine.poll();
        assert!(events.take().is_empty());
    }

    #[test]
    fn a_device_switch_waits_while_a_resume_is_held_back() {
        let (mut engine, handle, events) = exclusive_engine();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        engine.tx = tx;
        engine.tx.try_send(Cmd::RateTarget).expect("room for one");
        engine.sh.park(1, 96_000, 0, false);
        engine.poll();
        assert!(engine.reopen_pending(), "the resume is held back");
        let opens = handle.opens().len();

        // The switch is refused and leaves the reopened stream alone.
        assert_eq!(
            engine.switch_device(Some("dac".into())).err(),
            Some(PlaybackError::Busy)
        );
        assert!(handle.is_open(), "the reopened stream still plays");
        assert_eq!(
            handle.opens().len(),
            opens,
            "no stream was opened or closed"
        );
        assert!(events.take().is_empty(), "no OutputLost");

        // Once the decoder has its ring the reopen is reported and the switch goes through.
        assert!(matches!(rx.try_recv(), Ok(Cmd::RateTarget)));
        engine.poll();
        assert!(matches!(rx.try_recv(), Ok(Cmd::Resume { id: 1, .. })));
        assert_eq!(reopened(&events.take()), 1);
        engine
            .switch_device(Some("dac".into()))
            .expect("switch after the resume");
        assert_eq!(handle.opens().len(), opens + 1);
        assert!(handle.is_open());
        assert!(matches!(rx.try_recv(), Ok(Cmd::Reconfigure { .. })));
    }
}
