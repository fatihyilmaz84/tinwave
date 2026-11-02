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
use crate::output::{AudioOutput, OpenRequest, OutputStream};
use crate::reader::SourceOpener;
use crate::shared::{Mark, MarkKind, Shared};
use crate::types::CHANNELS;
use crate::{PlaybackError, Position, QueueItemId, Renderer, Track};

/// Seconds of audio the ring holds between the decoder and the callback.
const RING_SECONDS: usize = 1;
/// Position marks per second of heard audio.
const POSITION_MARKS_PER_SECOND: u32 = 10;
/// Commands that may wait for the decoder thread before calls fail with [`PlaybackError::Busy`].
const COMMAND_QUEUE: usize = 64;

/// Everything an [`Engine`] is wired to.
pub struct EngineConfig {
    /// Stream sample rate (stereo `f32`). Tracks must match it; resampling is T-0019.
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
    sample_rate: u32,
    device: Option<String>,
    gen: u64,
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
        let engine = Self {
            sh,
            tx,
            thread: Some(thread),
            output,
            stream: Some(stream),
            sink,
            clock,
            sample_rate,
            device,
            gen: 0,
            pending: None,
            cursor: None,
            seen_seq: None,
            announced: None,
            grid_base: 0,
            last_grid: 0,
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

    /// Where playback is, from frames the output has taken minus the output latency. Resolution is
    /// one audio callback. After a command this reports the command's target until the decoder has
    /// processed it.
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
        let mut view = self.view();
        if self.cursor.is_some() {
            if let Some(seen) = self.seen_seq {
                if let Some(next) = view.marks.iter().find(|m| m.seq > seen) {
                    view.played = view.played.min(next.start_frame);
                    view.marks.retain(|m| m.seq <= seen);
                }
            }
        }
        let (idx, frame) = resolve(&view.marks, view.played)?;
        Some(self.position_of(view.marks[idx].item.id, frame))
    }

    /// True from a `load`, `seek`, `stop` or `switch_device` call until the decoder thread has
    /// carried it out and the new stream is the one being rendered. `position` reports the
    /// command's target meanwhile.
    #[must_use]
    pub fn command_pending(&self) -> bool {
        self.unapplied().is_some()
    }

    /// Move to another output device (`None` is the system default) and resume at the exact frame
    /// that was playing. Frames already in the old device's buffer are not replayed.
    ///
    /// On error the engine has no open stream (the old one is already closed); call again.
    ///
    /// # Errors
    /// What the output reports; [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn switch_device(&mut self, device: Option<String>) -> Result<(), PlaybackError> {
        self.poll();
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
        let (producer, consumer) = ring(self.sample_rate);
        let gen = self.next_gen();
        let resume_item = resume.map(|r| r.0);
        let resume_frame = resume.map_or(0, |r| r.1);
        if let Err(error) = self.send(Cmd::Reconfigure {
            producer,
            item: resume_item,
            frame: resume_frame,
            gen,
        }) {
            // Nothing was sent, but the old stream is already closed.
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
        let request = OpenRequest {
            device: device.as_deref(),
            sample_rate: self.sample_rate,
        };
        match self
            .output
            .open(&request, Renderer::new(consumer, Arc::clone(&self.sh)))
        {
            Ok(stream) => {
                let info = stream.info();
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
        if self.unapplied().is_some() {
            return;
        }
        if let Some(done) = self.pending.take() {
            self.pending_applied(done);
        }
        let view = self.view();
        let rate_interval = u64::from(self.sample_rate / POSITION_MARKS_PER_SECOND).max(1);

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
                (Some(prev), MarkKind::Continuation) => {
                    let left_at = prev.offset + (mark.start_frame - prev.start_frame);
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
                let frame = last.offset + view.eos_total.saturating_sub(last.start_frame);
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
            let grid = heard.saturating_sub(self.grid_base) / rate_interval;
            if grid > self.last_grid {
                self.last_grid = grid;
                if let Some((idx, frame)) = resolve(&view.marks, heard) {
                    let position = self.position_of(view.marks[idx].item.id, frame);
                    self.emit(EventKind::PositionMark { position });
                }
            }
        }

        if let Some(seen) = self.seen_seq {
            self.sh.marks().retain(|m| m.seq >= seen);
        }
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
        View {
            consumed,
            played,
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

#[derive(Debug)]
struct View {
    consumed: u64,
    /// Frames heard: consumed minus output latency, or all consumed once the stream has ended.
    played: u64,
    eos_total: u64,
    eos_failed: bool,
    ended: bool,
    marks: Vec<Mark>,
}

/// Which mark holds output frame `played`, and the frame inside its track. Before the first mark's
/// start (the old stream's tail still playing after a seek) it is the first mark's offset.
fn resolve(marks: &[Mark], played: u64) -> Option<(usize, u64)> {
    match marks.iter().rposition(|m| m.start_frame <= played) {
        Some(i) => Some((i, marks[i].offset + (played - marks[i].start_frame))),
        None => marks.first().map(|m| (0, m.offset)),
    }
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
