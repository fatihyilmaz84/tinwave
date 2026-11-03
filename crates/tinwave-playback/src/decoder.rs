//! The decoder thread: reads tracks with Symphonia and keeps the ring full.
//!
//! This is the only thread that touches files. It never waits on the render callback except at the
//! seek handshake, and even there it keeps servicing commands.
//!
//! It is also the only thread that resamples (T-0019): tracks whose rate differs from the stream
//! rate go through a [`StreamResampler`] before they reach the ring, so the callback only ever
//! sees stream-rate frames.
//!
//! # Units
//! Everything the control side sees is in **stream frames**: seek targets, start frames, marks,
//! positions (`position_ms = frame * 1000 / stream_rate`). A track is read in its own **file
//! frames**; the two are the same number when the rates match and are converted (with
//! [`scale_frames`]) at the reader's seek and in the marks. [`GaplessInfo`](crate::GaplessInfo)
//! is the exception: it describes the file, so it stays in file frames.
//!
//! # Resampler lifetime
//! - Same-rate tracks that follow each other share one resampler (no restart at the boundary).
//! - A track of another rate: the old resampler is flushed into the ring first (its tail is the
//!   end of the previous track), then a new one starts.
//! - A seek, a load or a device switch restarts the stream, so the resampler is dropped with it
//!   (`reset_stream`); the first track after it starts a fresh one.
//! - The end of the queue flushes the tail before the end of stream is published.
//!
//! # Playback rate (ADR 0006, T-0027)
//! The chain is `reader -> [stretch] -> [resample] -> ring`. At rate exactly 1.0 there is no
//! stretch stage and the samples take the path they always took. At any other rate each track
//! goes through a [`StretchStage`] working at the file rate; its output is what the resampler
//! (if any) converts. The stage is flushed into the resampler at the end of every track, so the
//! next track's mark lands on an exact output frame and gapless playback stays gapless.
//!
//! A rate change is a flush at the audible position: the decoder raises `flush_req` (with
//! `flush_fade` set, so the renderer plays a 5 ms fade-out and fades the new stream in), waits for
//! the renderer's acknowledgement, and then reads how much the renderer has played
//! (`consumed`). The marks say which item and which source frame that is; the stream restarts
//! there at the new rate with a [`MarkKind::Rate`] mark, **keeping** the older marks (they describe
//! the output frames before the flush, which the control side may still be hearing). At most one
//! such flush happens per [`RATE_FLUSH_INTERVAL`]; a change that arrives sooner waits, and the
//! value current when the interval is over is the one applied, so the final value of a drag is
//! never lost. The interval is over when it has passed on the monotonic clock or on the engine's
//! injected clock (`Decoder::clock`, the same wall time in production), so a test ends it by
//! moving its manual clock instead of sleeping, and a wall clock set back never holds a change up
//! (`Decoder::throttle_wait`). While the acknowledgement of such a flush is awaited the thread
//! polls at `FLUSH_WAIT_MIN` and does not back off (`Decoder::wait_for_ack`): the listener hears
//! a gap until the new stream starts, and a running stream acknowledges within a callback. With
//! nothing audible (nothing loaded, or everything has been played) there is nothing to flush and
//! the rate is simply adopted.
//!
//! The restart reads the track again from the audible frame. An item that fails again there is
//! not reported twice (`State::reported`), and one that cannot be reopened at all leaves as
//! `Failed` (`State::heard`), not `Finished`.
//!
//! # Exclusive mode: the stream follows the file (T-0057)
//! In exclusive mode (`Shared::follow`) the output is not resampled to one rate: it is reopened
//! at the rate of each file, so the samples reach the device bit for bit. The decoder cannot
//! open streams, so when the next track's rate is not the stream's (`reopen_target`) it **parks**:
//! it publishes the rate it needs (`Shared::park`) and waits for the engine, which replaces the
//! stream when the old one has nothing left to play and answers with `Cmd::Resume` and a ring
//! for the new stream. Then the track is opened as usual.
//! - At a track boundary the tails of the track before (stretcher, resampler) are pushed first;
//!   the timeline continues in the new stream (`Parked::boundary`), the older marks stay, and the
//!   first mark of the new track is a `Continuation` like at any other boundary.
//! - A load, seek or device switch parks after its flush: there is nothing to wait for, and the
//!   command is answered (`applied_gen`) when the track has started in the new stream.
//! - Any command that restarts the stream cancels a park (`command_taken`). Appends and queue
//!   edits do not: the track is chosen when the stream is there.
//! - A rate the device refused (`Shared::refused`) is resampled to the engine's rate like in
//!   shared mode; marks carry the stream's rate (`Mark::out_rate`) because the frames the
//!   renderer counts are frames of whatever stream played them.
//! - Same-rate tracks never park, so gapless playback is untouched.

use std::collections::HashSet;
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rtrb::Producer;
use tinwave_core::{Clock, UtcMillis};

use crate::event::{EventKind, EventSink, PlaybackEvent};
use crate::rate_stage::StretchStage;
use crate::reader::{SourceOpener, TrackReader};
use crate::resample::{scale_frames, StreamResampler};
use crate::shared::{Mark, MarkKind, Shared};
use crate::types::CHANNELS;
use crate::{EditId, PlaybackError, QueueItemId, Track};

/// Commands from the control thread.
pub(crate) enum Cmd {
    /// Replace the queue and start at `queue[index]`, `frame` frames in. An empty queue stops.
    Load {
        queue: Vec<Arc<Track>>,
        index: usize,
        frame: u64,
        gen: u64,
    },
    /// Jump inside the current queue.
    Seek {
        item: QueueItemId,
        frame: u64,
        gen: u64,
    },
    /// Add tracks after the last queue entry.
    Append(Vec<Arc<Track>>),
    /// Replace everything after `head` with `items`, if `head` is the last item the decoder
    /// started. Answered with `QueueEdited` or `QueueEditRejected`.
    EditQueue {
        edit: EditId,
        head: QueueItemId,
        items: Vec<Arc<Track>>,
    },
    /// The output stream was re-created: use this ring and resume at `item`/`frame`.
    Reconfigure {
        producer: Producer<f32>,
        /// Sample rate of the new stream.
        rate: u32,
        /// `None` when nothing is loaded.
        item: Option<QueueItemId>,
        frame: u64,
        gen: u64,
    },
    /// The output stream was reopened at the rate the decoder asked for (`Shared::reopen`): use
    /// this ring and carry on with the track that was waiting. `id` names the request.
    Resume {
        producer: Producer<f32>,
        rate: u32,
        id: u32,
    },
    /// The rate target changed (it is in `Shared::rate_target`): wake the thread so it applies it.
    /// A full command queue loses nothing, the thread checks the target on every pass.
    RateTarget,
    /// Wake the thread so it notices the shutdown flag.
    Shutdown,
}

impl std::fmt::Debug for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Load { .. } => "Load",
            Self::Seek { .. } => "Seek",
            Self::Append(_) => "Append",
            Self::EditQueue { .. } => "EditQueue",
            Self::Reconfigure { .. } => "Reconfigure",
            Self::Resume { .. } => "Resume",
            Self::RateTarget => "RateTarget",
            Self::Shutdown => "Shutdown",
        })
    }
}

/// Idle poll interval while the ring is full. The ring holds one second, so this keeps it at
/// least ~97 % full while costing about 40 wakeups per second.
const FULL_RING_WAIT: Duration = Duration::from_millis(25);
/// Poll interval while waiting for the renderer to acknowledge a flush: starts short (a running
/// stream acks within a callback) and, for a command's flush, backs off, so a stream that is not
/// running (failed device switch) costs about 20 wakeups per second instead of 1000. A rate
/// flush never backs off (see `Decoder::wait_for_ack`).
const FLUSH_WAIT_MIN: Duration = Duration::from_millis(1);
const FLUSH_WAIT_MAX: Duration = Duration::from_millis(50);
/// At most one rate flush per this long (ADR 0006: 100 to 150 ms while a slider is dragged). Each
/// flush costs the listener a 15 ms dip, so a drag must not produce one per pixel.
pub(crate) const RATE_FLUSH_INTERVAL: Duration = Duration::from_millis(125);

pub(crate) struct Decoder {
    pub sh: Arc<Shared>,
    pub rx: Receiver<Cmd>,
    pub producer: Producer<f32>,
    pub opener: Arc<dyn SourceOpener>,
    pub sink: Arc<dyn EventSink>,
    pub clock: Arc<dyn Clock>,
    pub sample_rate: u32,
}

/// Decoder-thread state, kept apart from the immutable wiring above.
struct State {
    queue: Vec<Arc<Track>>,
    /// Index of the item being read, or of the next one to open when `reader` is `None`.
    cur: usize,
    /// Index of the last item that was opened (its mark is published), so that an item whose
    /// successors all failed to open is still the one a queue edit must name. `None` until a
    /// stream has started, and after a restart in which nothing opened (an edit is then refused
    /// as stale while nothing is audible).
    last_opened: Option<usize>,
    reader: Option<TrackReader>,
    /// Converts the file rate of `reader` (and of the tracks after it, while they share the rate)
    /// to the stream rate; `None` while the tracks already have the stream rate.
    resampler: Option<StreamResampler>,
    /// The playback rate of the stream being decoded (source frames per output frame): what the
    /// marks carry and `Shared::stream_rate` publishes. Changes only when the stream restarts,
    /// or while nothing is audible.
    rate: f64,
    /// The time stretcher for the file rate of the track being read, kept between tracks and
    /// restarts (it holds about a megabyte); `None` until a track needed one. It stretches the
    /// current track only while `is_active`, which is never at rate 1.0.
    stretch: Option<StretchStage>,
    /// Stretched audio on its way to the resampler.
    stretched: Vec<f32>,
    /// When the last rate flush began, on the monotonic clock and on `Decoder::clock` (the
    /// throttle's clocks).
    last_rate_flush: Option<(Instant, UtcMillis)>,
    /// Items whose failure has been reported since the last command restarted the stream. A rate
    /// restart reads the track again, so a file that is broken at some place fails there a second
    /// time; the listener's session must not see that as a second failure.
    reported: HashSet<QueueItemId>,
    /// The item that was audible when a rate restart began, while it is being reopened: if it
    /// cannot be, it has failed (its `TrackLeft` says so), whatever the items after it do.
    heard: Option<QueueItemId>,
    /// One decoded packet at the file rate, before the stretcher and the resampler.
    chunk: Vec<f32>,
    /// Stream-rate audio waiting for room in the ring.
    pending: Vec<f32>,
    pending_off: usize,
    /// Stream timeline: renderer-consumed frames at the last flush plus frames pushed since.
    written: u64,
    next_seq: u64,
    mark_kind: MarkKind,
    /// The item that started last failed mid-decode (copied into the next mark and the end of
    /// stream, so the control side can tell `Failed` from `Finished`).
    item_failed: bool,
    /// A flush was requested; once the renderer acks `req`, restart as `restart` says.
    awaiting: Option<Awaiting>,
    /// Sample rate of the output stream the ring feeds: `Decoder::sample_rate` until the engine
    /// opens a stream at a file's own rate (exclusive mode, T-0057).
    stream_rate: u32,
    /// Waiting for the engine to reopen the output stream at the rate of the track at `cur`.
    parked: Option<Parked>,
    /// A track needs another stream, but the tail of the one before it must reach the ring
    /// first; it parks once it has.
    park_req: Option<ParkRequest>,
    /// Identifies a park to the engine; never 0 for a park that has been made.
    park_id: u32,
}

/// The decoder is stopped before the track at `cur` until the output stream runs at `target`.
struct Parked {
    id: u32,
    /// Timeline frame to open the track at once the stream is there.
    frame: u64,
    /// The command (load, seek, device switch) to answer with `applied_gen` once the track has
    /// started.
    gen: Option<u64>,
    /// A track boundary or a revived queue: the old stream is still playing out, the timeline
    /// continues and the marks stay. Otherwise the stream was already restarted when it parked.
    boundary: bool,
    /// The kind of mark the track gets.
    kind: MarkKind,
    /// A rate flush to count once the track has started.
    rate_flush: bool,
}

/// What `open_current` found: the track needs a stream of another rate.
struct ParkRequest {
    target: u32,
    frame: u64,
}

/// The outcome of `Decoder::open_current`.
enum Opened {
    /// A reader is open and its mark is published.
    Started,
    /// The queue ran out.
    Exhausted,
    /// The track needs the output stream at `target`; nothing was published.
    Park { target: u32, frame: u64 },
}

impl State {
    /// `cur` is the item that started last (a reader is open on it, or a command is about to
    /// open it), rather than the next one to open. A decoder parked after a boundary has not
    /// started `cur` yet; one parked for a load or seek has.
    fn head_is_cur(&self) -> bool {
        self.reader.is_some()
            || self.awaiting.is_some()
            || self.parked.as_ref().is_some_and(|p| !p.boundary)
    }
}

struct Awaiting {
    req: u64,
    restart: Restart,
}

/// Where the stream starts again once the renderer acknowledged a flush.
enum Restart {
    /// A command named the place: `st.cur` at `frame` (stream frames); it is answered with `gen`.
    At { frame: u64, gen: u64 },
    /// A rate change: wherever the renderer got to, at the new rate.
    Audible,
}

impl Decoder {
    pub fn run(mut self) {
        let mut st = State {
            queue: Vec::new(),
            cur: 0,
            last_opened: None,
            reader: None,
            resampler: None,
            rate: 1.0,
            stretch: None,
            stretched: Vec::new(),
            last_rate_flush: None,
            reported: HashSet::new(),
            heard: None,
            chunk: Vec::new(),
            pending: Vec::new(),
            pending_off: 0,
            written: 0,
            next_seq: 0,
            mark_kind: MarkKind::Initial,
            item_failed: false,
            awaiting: None,
            stream_rate: self.sample_rate,
            parked: None,
            park_req: None,
            park_id: 0,
        };
        let mut flush_wait = FLUSH_WAIT_MIN;
        loop {
            if self.sh.shutdown.load(Acquire) {
                return;
            }
            loop {
                match self.rx.try_recv() {
                    Ok(cmd) => {
                        if !self.accept(&mut st, cmd) {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            // A rate change begins a flush, so it is looked at before the flush is waited for.
            let rate_wait = self.service_rate(&mut st);

            if let Some(awaiting) = &st.awaiting {
                if self.sh.flush_ack.load(Acquire) != awaiting.req {
                    match Self::wait_for_ack(&self.rx, &awaiting.restart, &mut flush_wait) {
                        Ok(Some(cmd)) => {
                            if !self.accept(&mut st, cmd) {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(()) => return,
                    }
                    continue;
                }
                flush_wait = FLUSH_WAIT_MIN;
                if let Some(awaiting) = st.awaiting.take() {
                    match awaiting.restart {
                        Restart::At { frame, gen } => self.restart(&mut st, frame, gen),
                        Restart::Audible => self.restart_audible(&mut st),
                    }
                }
            }

            if st.parked.is_some() {
                // Nothing to decode until the engine has reopened the stream (`Cmd::Resume`) or a
                // command makes the wait pointless.
                match Self::wait_for_command(&self.rx, None, None) {
                    Ok(Some(cmd)) => {
                        if !self.accept(&mut st, cmd) {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => return,
                }
                continue;
            }

            if st.pending_off >= st.pending.len() {
                st.pending.clear();
                st.pending_off = 0;
                if !self.fill_pending(&mut st) {
                    // End of queue: sleep until a command arrives (or a deferred rate change is
                    // due: the tail of the last track may still be playing).
                    match Self::wait_for_command(&self.rx, None, rate_wait) {
                        Ok(Some(cmd)) => {
                            if !self.accept(&mut st, cmd) {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(_) => return,
                    }
                    continue;
                }
            }

            // Whole frames only, so the stream timeline never splits a frame.
            let free = self.producer.slots() & !1;
            let take = free.min(st.pending.len() - st.pending_off);
            if take == 0 {
                match Self::wait_for_command(&self.rx, Some(FULL_RING_WAIT), rate_wait) {
                    Ok(Some(cmd)) => {
                        if !self.accept(&mut st, cmd) {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => return,
                }
                continue;
            }
            let (pushed, _) = self
                .producer
                .push_partial_slice(&st.pending[st.pending_off..st.pending_off + take]);
            st.pending_off += pushed.len();
            st.written += (pushed.len() / CHANNELS) as u64;
            // Audio is in the ring now: from here an empty ring is a real underrun again. (Cleared
            // after the push, so the renderer never sees "not buffering" with nothing to play.)
            if self.sh.buffering.load(Relaxed) {
                self.sh.buffering.store(false, Release);
            }
        }
    }

    /// Handle one command. `false` means the thread should exit.
    fn accept(&mut self, st: &mut State, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Shutdown => return false,
            Cmd::RateTarget => {}
            Cmd::Load {
                queue,
                index,
                frame,
                gen,
            } => {
                self.drop_reader(st);
                st.queue = queue;
                st.cur = index;
                self.command_taken(st, gen, true);
                self.begin_flush(st, Restart::At { frame, gen }, false);
            }
            Cmd::Seek { item, frame, gen } => match st.queue.iter().position(|t| t.id == item) {
                Some(index) => {
                    self.drop_reader(st);
                    st.cur = index;
                    self.command_taken(st, gen, true);
                    self.begin_flush(st, Restart::At { frame, gen }, false);
                }
                None => {
                    self.command_taken(st, gen, false);
                    self.emit(EventKind::SeekFailed {
                        item,
                        frame,
                        error: PlaybackError::UnknownItem(item),
                    });
                    self.sh.applied_gen.fetch_max(gen, Release);
                }
            },
            Cmd::Append(items) => {
                if items.is_empty() {
                    return true;
                }
                st.queue.extend(items);
                self.revive_if_ended(st);
            }
            Cmd::EditQueue { edit, head, items } => self.edit_queue(st, edit, head, items),
            Cmd::Reconfigure {
                producer,
                rate,
                item,
                frame,
                gen,
            } => {
                self.producer = producer;
                st.stream_rate = rate;
                self.drop_reader(st);
                self.command_taken(st, gen, true);
                // No item means no track is current (stopped, or the queue ended): keep `cur`, which
                // is the end of the queue or the first appended track that has not been heard yet.
                // A named item that is no longer queued ends the queue.
                if let Some(id) = item {
                    st.cur = st
                        .queue
                        .iter()
                        .position(|t| t.id == id)
                        .unwrap_or(st.queue.len());
                }
                // The old stream is gone, so nothing can race: align the handshake and restart.
                let req = self.sh.flush_req.load(Acquire);
                self.sh.flush_ack.store(req, Release);
                st.awaiting = None;
                self.restart(st, frame, gen);
            }
            Cmd::Resume { producer, rate, id } => self.resume(st, producer, rate, id),
        }
        true
    }

    /// A restarting command (load, seek, device switch, stop) was taken from the queue: it
    /// replaces whatever the decoder was waiting for (`restarts`), and the engine is told which
    /// command the decoder has got to. The park is withdrawn first, so an engine that sees the
    /// new generation never sees the old request after it.
    fn command_taken(&mut self, st: &mut State, gen: u64, restarts: bool) {
        if restarts {
            st.park_req = None;
            if let Some(parked) = st.parked.take() {
                self.sh.unpark(parked.id);
            }
        }
        self.sh.reopen_gen.store(gen, Release);
    }

    /// The engine reopened the output stream for the request `id`: carry on with the track that
    /// was waiting for it, in the new ring.
    fn resume(&mut self, st: &mut State, producer: Producer<f32>, rate: u32, id: u32) {
        let parked = match st.parked.take() {
            Some(parked) if parked.id == id => parked,
            other => {
                // The engine only answers the request of its latest command, and only commands
                // cancel a request: this cannot happen. The stream it opened has nobody to feed it.
                st.parked = other;
                tracing::error!(
                    id,
                    "the output was reopened for a request that is not pending"
                );
                return;
            }
        };
        self.producer = producer;
        st.stream_rate = rate;
        self.sh.unpark(id);
        if parked.boundary {
            // The old stream has played out; the timeline goes on in the new one, and the marks
            // that describe the old one stay.
            let failed = st.item_failed;
            self.reset_stream(st, true);
            st.item_failed = failed;
        }
        st.mark_kind = parked.kind;
        if self.begin(st, parked.frame, parked.gen, parked.rate_flush) {
            return;
        }
        if let Some(gen) = parked.gen {
            self.sh.applied_gen.fetch_max(gen, Release);
        }
        if parked.rate_flush {
            self.sh.rate_flushes.fetch_add(1, Release);
        }
    }

    /// Open `st.cur` at `frame` (timeline frames) and publish its mark; the queue running out
    /// ends the stream. `true` when the track needs another output stream first: the decoder
    /// is then parked (`gen` and `rate_flush` are what to answer once it has started).
    fn begin(&mut self, st: &mut State, frame: u64, gen: Option<u64>, rate_flush: bool) -> bool {
        let outcome = if st.cur >= st.queue.len() {
            Opened::Exhausted
        } else {
            self.open_current(st, frame)
        };
        match outcome {
            Opened::Started => false,
            Opened::Exhausted => {
                self.finish_queue(st);
                false
            }
            Opened::Park { target, frame } => {
                self.park(st, target, frame, gen, false, rate_flush);
                true
            }
        }
    }

    /// Stop decoding until the engine has reopened the output stream at `target`. `boundary`:
    /// the tail of the old stream is still playing (everything pushed so far is in `written`),
    /// otherwise the stream was restarted just now and there is nothing left to wait for.
    fn park(
        &mut self,
        st: &mut State,
        target: u32,
        frame: u64,
        gen: Option<u64>,
        boundary: bool,
        rate_flush: bool,
    ) {
        st.park_id = st.park_id.wrapping_add(1).max(1);
        st.parked = Some(Parked {
            id: st.park_id,
            frame,
            gen,
            boundary,
            kind: st.mark_kind,
            rate_flush,
        });
        // Waiting for a stream is buffering, not an underrun.
        self.sh.buffering.store(true, Release);
        self.sh.park(st.park_id, target, st.written, boundary);
    }

    /// The rate the output stream has to run at for a track of `file_rate` to start, if it is
    /// not the one it runs at. Only in exclusive mode (the engine publishes `follow`): the
    /// stream follows the file, unless the device refused that rate before, when the track is
    /// resampled to the engine's own rate instead.
    fn reopen_target(&self, st: &State, file_rate: u32) -> Option<u32> {
        if !self.sh.follow.load(Acquire) {
            return None;
        }
        let target = if self.sh.refused().contains(&file_rate) {
            self.sample_rate
        } else {
            file_rate
        };
        (target != st.stream_rate).then_some(target)
    }

    /// The queue had ended and now has entries the decoder has not opened: continue decoding into
    /// the same ring (gapless).
    fn revive_if_ended(&mut self, st: &State) {
        if st.awaiting.is_none()
            && st.reader.is_none()
            && self.sh.eos.load(Relaxed)
            && st.cur < st.queue.len()
        {
            // Buffering is announced first, so a renderer that sees the revived stream
            // (`eos` false) also sees that its empty ring is not yet an underrun.
            self.sh.buffering.store(true, Release);
            self.sh.eos.store(false, Release);
        }
    }

    /// Replace the entries after `head`, which must be the last item this thread started: its
    /// audio is (partly) in the ring and its mark is published, while everything after it is still
    /// only a list. Anything the decoder has already started cannot be edited, because its
    /// audio is in the ring, so an edit built on an older head is refused, never applied to the
    /// wrong item.
    fn edit_queue(
        &mut self,
        st: &mut State,
        edit: EditId,
        head: QueueItemId,
        items: Vec<Arc<Track>>,
    ) {
        match Self::edit_base(st, head, &items) {
            Ok(base) => {
                st.queue.truncate(base + 1);
                // An id that leaves the queue may come back as a new entry: it reports afresh.
                let queued: HashSet<QueueItemId> = st.queue.iter().map(|t| t.id).collect();
                st.reported.retain(|id| queued.contains(id));
                st.queue.extend(items);
                if !st.head_is_cur() {
                    // Past the end of the list (the items after `head` failed to open): the next
                    // item to open is the first of the new tail.
                    st.cur = base + 1;
                }
                self.revive_if_ended(st);
                self.emit(EventKind::QueueEdited { edit, head });
            }
            Err(error) => self.emit(EventKind::QueueEditRejected { edit, head, error }),
        }
    }

    /// Index of `head` in the queue, if it is the last item started and `items` do not repeat an
    /// id of the part that stays.
    fn edit_base(
        st: &State,
        head: QueueItemId,
        items: &[Arc<Track>],
    ) -> Result<usize, PlaybackError> {
        // While a reader is open (or a command is about to open one) `cur` is the item being
        // read; once the queue is exhausted the last item started is the last one opened (not the
        // one before `cur`, which may be an item that failed to open).
        let started = if st.head_is_cur() {
            Some(st.cur)
        } else {
            st.last_opened
        };
        let base = started.filter(|&i| i < st.queue.len());
        let current = base.map(|i| st.queue[i].id);
        let stale = PlaybackError::StaleQueue {
            expected: head,
            current,
        };
        match base {
            Some(i) if st.queue[i].id == head => {
                let kept: HashSet<QueueItemId> = st.queue[..=i].iter().map(|t| t.id).collect();
                if items.iter().any(|t| kept.contains(&t.id)) {
                    return Err(PlaybackError::InvalidArgument(
                        "queue item ids must be unique",
                    ));
                }
                Ok(i)
            }
            _ => Err(stale),
        }
    }

    /// Ask the renderer to drop the ring. `fade`: a rate change, so it plays a short fade-out
    /// first (see `Shared::flush_fade`).
    fn begin_flush(&mut self, st: &mut State, restart: Restart, fade: bool) {
        debug_assert!(
            st.reader.is_none(),
            "readers are dropped through drop_reader"
        );
        st.pending.clear();
        st.pending_off = 0;
        // Before the request is raised: the renderer reads it after seeing the request.
        self.sh.flush_fade.store(fade, Release);
        let req = self.sh.flush_req.fetch_add(1, AcqRel) + 1;
        st.awaiting = Some(Awaiting { req, restart });
    }

    /// Start a fresh stream timeline at the renderer's current position and open `st.cur`.
    fn restart(&mut self, st: &mut State, frame: u64, gen: u64) {
        self.reset_stream(st, false);
        // A command (load, seek, device switch) starts over: failures are reported afresh.
        st.reported.clear();
        st.mark_kind = MarkKind::Initial;
        if self.begin(st, frame, Some(gen), false) {
            // The command is answered once the stream has been reopened and the track started.
            return;
        }
        // Published last: the control thread then sees the first mark (or the end of stream).
        self.sh.applied_gen.fetch_max(gen, Release);
    }

    /// Apply the rate target if it differs from the rate in use. Called on every pass of the
    /// thread's loop. Returns how long until a change that has to wait for the throttle may be
    /// applied (the loop's waits must not sleep past it).
    fn service_rate(&mut self, st: &mut State) -> Option<Duration> {
        if st.awaiting.is_some() || st.parked.is_some() || st.park_req.is_some() {
            // A flush or a stream swap is under way; the target is looked at again after it.
            return None;
        }
        let target = self.sh.rate_target();
        if target == st.rate {
            return None;
        }
        if !self.audible() {
            // Nothing in the ring that anyone will hear: there is no stream to flush.
            self.adopt_rate(st, target);
            return None;
        }
        let now = (Instant::now(), self.clock.now());
        if let Some(last) = st.last_rate_flush {
            let wait = Self::throttle_wait(last, now);
            if !wait.is_zero() {
                return Some(wait);
            }
        }
        st.last_rate_flush = Some(now);
        self.drop_reader(st);
        self.begin_flush(st, Restart::Audible, true);
        None
    }

    /// How much longer a rate flush has to wait after the one at `last`, with `now` on the same
    /// clocks: the interval is over when either clock says so. The injected clock is wall time and
    /// can be set back, which says nothing (the monotonic clock then decides); set forward it
    /// ends the interval early, which costs at most one extra flush.
    fn throttle_wait(last: (Instant, UtcMillis), now: (Instant, UtcMillis)) -> Duration {
        let monotonic = RATE_FLUSH_INTERVAL.saturating_sub(now.0.saturating_duration_since(last.0));
        let injected = match u64::try_from(now.1.as_i64().saturating_sub(last.1.as_i64())) {
            Ok(elapsed_ms) => RATE_FLUSH_INTERVAL.saturating_sub(Duration::from_millis(elapsed_ms)),
            Err(_) => RATE_FLUSH_INTERVAL,
        };
        monotonic.min(injected)
    }

    /// Wait for a command, or for the poll interval to pass, while the renderer's acknowledgement
    /// of a flush is outstanding. `backoff` is the interval a command's flush has reached: it
    /// doubles on every timeout (the stream may not be running at all) and starts over with a
    /// command. A rate flush is always polled at the minimum: the stream is running, the listener
    /// hears a gap until the acknowledgement is noticed, and a backoff would stretch that gap by
    /// up to the backoff. `Ok(None)` is a timeout; `Err` means the control side is gone.
    fn wait_for_ack(
        rx: &Receiver<Cmd>,
        restart: &Restart,
        backoff: &mut Duration,
    ) -> Result<Option<Cmd>, ()> {
        let poll = match restart {
            Restart::At { .. } => *backoff,
            Restart::Audible => FLUSH_WAIT_MIN,
        };
        match rx.recv_timeout(poll) {
            Ok(cmd) => {
                *backoff = FLUSH_WAIT_MIN;
                Ok(Some(cmd))
            }
            Err(RecvTimeoutError::Timeout) => {
                if matches!(restart, Restart::At { .. }) {
                    *backoff = (*backoff * 2).min(FLUSH_WAIT_MAX);
                }
                Ok(None)
            }
            Err(RecvTimeoutError::Disconnected) => Err(()),
        }
    }

    /// Whether the stream this thread has been feeding still has audio that has not been played:
    /// it has a mark, and the end of stream (if reached) is not yet behind the renderer.
    fn audible(&self) -> bool {
        let ended = self.sh.eos.load(Acquire)
            && self.sh.consumed.load(Acquire) >= self.sh.eos_total.load(Acquire);
        !ended && !self.sh.marks().is_empty()
    }

    /// Use `rate` for everything decoded from now on. Only while the ring is empty: the renderer
    /// reads the rate of the frames it pops, so it must not change under audio that is queued.
    fn adopt_rate(&mut self, st: &mut State, rate: f64) {
        st.rate = rate;
        self.sh.stream_rate.store(rate.to_bits(), Release);
    }

    /// The renderer acknowledged a rate flush: start again where it got to, at the new rate.
    ///
    /// `consumed` is exact now (the acknowledgement is stored after the renderer's last update of
    /// it, and the ring is empty), and the marks turn it into an item and a source frame. The marks
    /// from before the flush stay: the control side still resolves positions it has not yet heard
    /// (the device buffer) through them. The ones after `consumed` are the audio that was just
    /// thrown away, and go.
    fn restart_audible(&mut self, st: &mut State) {
        let consumed = self.sh.consumed.load(Acquire);
        let at = {
            let mut marks = self.sh.marks();
            let at = marks
                .iter()
                .rev()
                .find(|m| m.start_frame <= consumed)
                .or_else(|| marks.front())
                .map(|m| (m.item.id, m.source_frame_at(consumed)));
            marks.retain(|m| m.start_frame <= consumed);
            at
        };
        let Some((item, frame)) = at else {
            // Everything was cleared (a stop that overtook the flush): nothing to resume.
            self.reset_stream(st, false);
            self.finish_queue(st);
            self.sh.rate_flushes.fetch_add(1, Release);
            return;
        };
        st.cur = st
            .queue
            .iter()
            .position(|t| t.id == item)
            .unwrap_or(st.queue.len());
        self.reset_stream(st, true);
        st.mark_kind = MarkKind::Rate;
        st.heard = Some(item);
        let parked = self.begin(st, frame, None, true);
        st.heard = None;
        if parked {
            return;
        }
        // Last: whoever sees the count has the new rate and the first mark of the new stream.
        self.sh.rate_flushes.fetch_add(1, Release);
    }

    /// Forget the old stream: `keep_marks` keeps the marks already published (a rate restart
    /// continues the output timeline), otherwise the new stream is a new timeline.
    fn reset_stream(&mut self, st: &mut State, keep_marks: bool) {
        debug_assert!(
            st.reader.is_none(),
            "readers are dropped through drop_reader"
        );
        st.item_failed = false;
        st.last_opened = None;
        // A new stream timeline: whatever the resampler or the stretcher still held belonged to
        // the old one.
        st.resampler = None;
        if let Some(stage) = st.stretch.as_mut() {
            stage.stop();
        }
        st.pending.clear();
        st.pending_off = 0;
        st.written = self.sh.consumed.load(Acquire);
        if !keep_marks {
            self.sh.marks().clear();
        }
        // The ring is empty here (the renderer acknowledged the flush, or this is a new ring), so
        // the rate may change: the renderer reads it only for frames pushed after this.
        let target = self.sh.rate_target();
        self.adopt_rate(st, target);
        self.sh.buffering.store(false, Release);
        self.sh.eos.store(false, Release);
    }

    fn finish_queue(&mut self, st: &mut State) {
        self.sh.eos_total.store(st.written, Release);
        self.sh.eos_failed.store(st.item_failed, Release);
        self.sh.buffering.store(false, Release);
        self.sh.eos.store(true, Release);
    }

    /// Open `st.cur` at `frame`, moving on to later queue entries when one fails. A track that
    /// needs the output stream at another rate is not started: see [`Opened::Park`].
    fn open_current(&mut self, st: &mut State, mut frame: u64) -> Opened {
        while let Some(track) = st.queue.get(st.cur).cloned() {
            match TrackReader::open(self.opener.as_ref(), &track) {
                Ok(mut reader) => {
                    // The stretcher only touches itself, so it is made ready before the step that
                    // changes the stream (the resampler), which must be the last thing that can
                    // fail.
                    let file_rate = reader.sample_rate();
                    if let Some(target) = self.reopen_target(st, file_rate) {
                        // Nothing has been read: the track is opened again once the stream is.
                        return Opened::Park { target, frame };
                    }
                    let stage = match Self::stage_for(st, file_rate) {
                        Ok(stage) => stage,
                        Err(error) => {
                            self.track_failed(st, track.id, error);
                            st.cur += 1;
                            frame = 0;
                            continue;
                        }
                    };
                    // A stretcher warms up on audio before the start: the reader begins earlier.
                    let wanted = stage.as_ref().map_or(0, |s| s.preroll_frames() as u64);
                    let target = if frame > 0 {
                        self.file_frame(&reader, frame)
                    } else {
                        0
                    };
                    let preroll = if target > 0 || wanted > 0 {
                        match reader.seek_with_preroll(target, wanted) {
                            Ok(preroll) => preroll,
                            Err(error) => {
                                // The stage is still unused: keep it for the next track.
                                st.stretch = stage;
                                Self::heard_failed(st, track.id);
                                self.emit(EventKind::SeekFailed {
                                    item: track.id,
                                    frame,
                                    error,
                                });
                                st.cur += 1;
                                frame = 0;
                                continue;
                            }
                        }
                    } else {
                        0
                    };
                    if let Some(mut stage) = stage {
                        if let Err(error) = stage.begin(st.rate, preroll as usize) {
                            self.track_failed(st, track.id, error);
                            st.cur += 1;
                            frame = 0;
                            continue;
                        }
                        st.stretch = Some(stage);
                    }
                    if let Err(error) = self.use_rate(st, file_rate) {
                        if let Some(stage) = st.stretch.as_mut() {
                            stage.stop();
                        }
                        self.track_failed(st, track.id, error);
                        st.cur += 1;
                        frame = 0;
                        continue;
                    }
                    let seq = st.next_seq;
                    st.next_seq += 1;
                    self.sh.marks().push_back(Mark {
                        seq,
                        start_frame: Self::stream_end(st),
                        item: track,
                        offset: frame,
                        kind: st.mark_kind,
                        prev_failed: st.item_failed,
                        rate: st.rate,
                        out_rate: st.stream_rate,
                        timeline_rate: self.sample_rate,
                    });
                    st.item_failed = false;
                    st.mark_kind = MarkKind::Continuation;
                    st.last_opened = Some(st.cur);
                    st.reader = Some(reader);
                    return Opened::Started;
                }
                Err(error) => {
                    self.track_failed(st, track.id, error);
                    st.cur += 1;
                    frame = 0;
                }
            }
        }
        Opened::Exhausted
    }

    /// An item could not be started (or read to its end). Reported once per item until a command
    /// restarts the stream (`State::reported`). If it is the item a rate restart is reopening,
    /// it leaves as failed.
    fn track_failed(&mut self, st: &mut State, item: QueueItemId, error: PlaybackError) {
        Self::heard_failed(st, item);
        if st.reported.insert(item) {
            self.emit(EventKind::TrackFailed { item, error });
        }
    }

    /// `item` could not be reopened by a rate restart: the listener was hearing it, so it leaves
    /// as `Failed`. (The flag is read by the mark of whichever item starts next, or by the end of
    /// the stream.)
    fn heard_failed(st: &mut State, item: QueueItemId) {
        if st.heard == Some(item) {
            st.item_failed = true;
            st.heard = None;
        }
    }

    /// The stretch stage for a track of `file_rate` that starts now, taken out of `st` (the caller
    /// puts it back once it has begun it), or `None` at rate 1.0, where there is no stretching. The
    /// stage is kept from track to track and rebuilt only when the file rate changes.
    fn stage_for(st: &mut State, file_rate: u32) -> Result<Option<StretchStage>, PlaybackError> {
        if st.rate == 1.0 {
            if let Some(stage) = st.stretch.as_mut() {
                stage.stop();
            }
            return Ok(None);
        }
        match st.stretch.take() {
            Some(stage) if stage.sample_rate() == file_rate => Ok(Some(stage)),
            _ => StretchStage::new(file_rate).map(Some),
        }
    }

    /// `frame` (stream frames) as a frame of the file `reader` is reading.
    fn file_frame(&self, reader: &TrackReader, frame: u64) -> u64 {
        let rate = reader.sample_rate();
        if rate == self.sample_rate {
            return frame;
        }
        let file_frame = scale_frames(frame, self.sample_rate, rate);
        // Rounding must not turn the very last stream frame into a frame past the file's end
        // (96 kHz -> 48 kHz: 2 * round(n / 2) can be n + 1).
        match reader.length() {
            Some(length)
                if file_frame > length && frame <= scale_frames(length, rate, self.sample_rate) =>
            {
                length
            }
            _ => file_frame,
        }
    }

    /// Make the conversion stage fit a track of `rate` that is about to start. Same-rate tracks
    /// share their resampler; a different rate flushes the old one into `pending` (the end of the
    /// previous track) and starts another. Fails, changing nothing, when the rate cannot be
    /// converted.
    fn use_rate(&mut self, st: &mut State, rate: u32) -> Result<(), PlaybackError> {
        let wanted = match &st.resampler {
            Some(current) => current.in_rate(),
            None => st.stream_rate,
        };
        if rate == wanted {
            return Ok(());
        }
        // Built before the old one is flushed, so a refused rate leaves the stream untouched.
        let next = if rate == st.stream_rate {
            None
        } else {
            Some(StreamResampler::new(rate, st.stream_rate)?)
        };
        if let Err(error) = self.flush_resampler(st) {
            // The old track's tail is lost: that track failed, not the one that starts now.
            self.flush_failed(st, error);
        }
        // The flush moved the end of the timeline: the new resampler's output starts there.
        st.resampler = next.map(|resampler| resampler.start_at(Self::stream_end(st)));
        Ok(())
    }

    /// Stream frame the next decoded frame lands on: the end of everything queued so far.
    fn stream_end(st: &State) -> u64 {
        match &st.resampler {
            Some(resampler) => resampler.position(),
            None => st.written + ((st.pending.len() - st.pending_off) / CHANNELS) as u64,
        }
    }

    /// Move the resampler's tail into `pending`. Returns whether it added any audio.
    fn flush_resampler(&mut self, st: &mut State) -> Result<bool, PlaybackError> {
        let Some(resampler) = st.resampler.take() else {
            return Ok(false);
        };
        let before = st.pending.len();
        resampler.finish(&mut st.pending)?;
        Ok(st.pending.len() > before)
    }

    /// The tail of the last track that was read could not be converted, so that track ends early:
    /// it is reported as failed (and its `TrackLeft` will say so). The last track read is the last
    /// one opened, which is not `cur - 1` when the items after it failed to open.
    fn flush_failed(&mut self, st: &mut State, error: PlaybackError) {
        if let Some(item) = st.last_opened.and_then(|i| st.queue.get(i)).map(|t| t.id) {
            self.track_failed(st, item, error);
        }
        st.item_failed = true;
    }

    /// The conversion stage: file-rate audio in `st.chunk` to stream-rate audio in `st.pending`.
    /// At a rate other than 1.0 the stretcher (file rate) runs first and the resampler converts
    /// its output; at 1.0 the audio goes straight on.
    fn convert_chunk(st: &mut State) -> Result<(), PlaybackError> {
        match st.stretch.as_mut().filter(|stage| stage.is_active()) {
            Some(stage) => {
                st.stretched.clear();
                stage.push(&st.chunk, &mut st.stretched)?;
                Self::forward(&mut st.resampler, &st.stretched, &mut st.pending)
            }
            None => Self::forward(&mut st.resampler, &st.chunk, &mut st.pending),
        }
    }

    /// Resample `samples` into `pending`, or copy them when the file has the stream's rate.
    fn forward(
        resampler: &mut Option<StreamResampler>,
        samples: &[f32],
        pending: &mut Vec<f32>,
    ) -> Result<(), PlaybackError> {
        match resampler.as_mut() {
            Some(resampler) => resampler.push(samples, pending),
            None => {
                pending.extend_from_slice(samples);
                Ok(())
            }
        }
    }

    /// The track that was being read is over: run the stretcher's tail out into the conversion
    /// stage, so the next track's mark lands where this one really ends.
    fn flush_stretch(st: &mut State) -> Result<(), PlaybackError> {
        let Some(stage) = st.stretch.as_mut().filter(|stage| stage.is_active()) else {
            return Ok(());
        };
        st.stretched.clear();
        stage.finish(&mut st.stretched)?;
        Self::forward(&mut st.resampler, &st.stretched, &mut st.pending)
    }

    /// Decode more audio into `st.pending`, advancing through the queue. `false` at the end.
    fn fill_pending(&mut self, st: &mut State) -> bool {
        loop {
            if let Some(request) = st.park_req.take() {
                // The tail of the track before has reached the ring: wait for the stream.
                self.park(st, request.target, request.frame, None, true, false);
                return false;
            }
            if st.reader.is_none() {
                match self.open_current(st, 0) {
                    Opened::Started => {}
                    Opened::Park { target, frame } => {
                        // The audio of the track before is the end of the old stream: the
                        // resampler's tail joins the stretcher's in `pending`, and all of it is
                        // pushed before the decoder parks.
                        if let Err(error) = self.flush_resampler(st) {
                            self.flush_failed(st, error);
                        }
                        st.park_req = Some(ParkRequest { target, frame });
                        if st.pending_off < st.pending.len() {
                            return true;
                        }
                        continue;
                    }
                    Opened::Exhausted => {
                        // The last track's tail is still inside the resampler, and the stretcher's
                        // tail (flushed with the track) may be waiting in `pending`: it is audio, so
                        // it goes out before the end of the stream is published.
                        if let Err(error) = self.flush_resampler(st) {
                            self.flush_failed(st, error);
                        }
                        if st.pending_off < st.pending.len() {
                            return true;
                        }
                        self.finish_queue(st);
                        return false;
                    }
                }
            }
            let Some(reader) = st.reader.as_mut() else {
                continue;
            };
            st.chunk.clear();
            let result = match reader.next_chunk(&mut st.chunk) {
                Ok(true) => Self::convert_chunk(st).map(|()| true),
                other => other,
            };
            match result {
                // A packet can leave nothing for the ring yet (the resampler is still filling).
                Ok(true) if st.pending.is_empty() => {}
                Ok(true) => return true,
                Ok(false) => self.finish_item(st),
                Err(error) => {
                    if let Some(item) = st.queue.get(st.cur).map(|t| t.id) {
                        self.track_failed(st, item, error);
                    }
                    st.item_failed = true;
                    self.finish_item(st);
                }
            }
        }
    }

    fn finish_item(&mut self, st: &mut State) {
        if let Err(error) = Self::flush_stretch(st) {
            self.flush_failed(st, error);
        }
        self.drop_reader(st);
        st.cur += 1;
    }

    /// The only way a reader goes away: corrupt packets it skipped are reported first, whether the
    /// track ended, failed, was seeked inside, skipped or restarted for a device switch.
    /// Must run while `st.cur` still names the reader's item.
    fn drop_reader(&mut self, st: &mut State) {
        let Some(mut reader) = st.reader.take() else {
            return;
        };
        let packets = reader.take_corrupt_packets();
        if packets > 0 {
            if let Some(track) = st.queue.get(st.cur) {
                self.emit(EventKind::CorruptPackets {
                    item: track.id,
                    packets,
                });
            }
        }
    }

    /// Wait for the next command: up to `base` (forever if `None`), but never past `cap`, the
    /// time until a deferred rate change is due. `Ok(None)` is a timeout; `Err` means the control
    /// side is gone.
    fn wait_for_command(
        rx: &Receiver<Cmd>,
        base: Option<Duration>,
        cap: Option<Duration>,
    ) -> Result<Option<Cmd>, ()> {
        let limit = match (base, cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match limit {
            Some(limit) => match rx.recv_timeout(limit) {
                Ok(cmd) => Ok(Some(cmd)),
                Err(RecvTimeoutError::Timeout) => Ok(None),
                Err(RecvTimeoutError::Disconnected) => Err(()),
            },
            None => rx.recv().map(Some).map_err(|_| ()),
        }
    }

    fn emit(&self, kind: EventKind) {
        self.sink.emit(PlaybackEvent {
            at: self.clock.now(),
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tinwave_core::{ManualClock, UtcMillis};

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

    fn track(id: u64) -> Arc<Track> {
        Arc::new(Track {
            id: QueueItemId(id),
            path: std::path::PathBuf::from("unused.wav"),
            gapless: None,
        })
    }

    fn state(queue: Vec<Arc<Track>>, cur: usize, last_opened: Option<usize>) -> State {
        State {
            queue,
            cur,
            last_opened,
            reader: None,
            resampler: None,
            rate: 1.0,
            stretch: None,
            stretched: Vec::new(),
            last_rate_flush: None,
            reported: HashSet::new(),
            heard: None,
            chunk: Vec::new(),
            pending: Vec::new(),
            pending_off: 0,
            written: 0,
            next_seq: 0,
            mark_kind: MarkKind::Initial,
            item_failed: false,
            awaiting: None,
            stream_rate: 48_000,
            parked: None,
            park_req: None,
            park_id: 0,
        }
    }

    fn decoder_on(sample_rate: u32) -> Decoder {
        let (_tx, rx) = std::sync::mpsc::channel();
        let (producer, _consumer) = rtrb::RingBuffer::new(8);
        Decoder {
            sh: Shared::new(1.0),
            rx,
            producer,
            opener: Arc::new(crate::FsOpener),
            sink: Arc::new(Events::default()),
            clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
            sample_rate,
        }
    }

    #[test]
    fn the_stream_follows_a_file_only_in_exclusive_mode() {
        let decoder = decoder_on(48_000);
        let mut st = state(Vec::new(), 0, None);
        st.stream_rate = 44_100;
        // Shared mode: every rate is converted, none asks for another stream.
        assert_eq!(decoder.reopen_target(&st, 96_000), None);
        decoder.sh.follow.store(true, Release);
        assert_eq!(decoder.reopen_target(&st, 96_000), Some(96_000));
        assert_eq!(
            decoder.reopen_target(&st, 44_100),
            None,
            "the stream already has it"
        );
    }

    #[test]
    fn a_refused_rate_is_converted_to_the_engines_rate_instead() {
        let decoder = decoder_on(48_000);
        decoder.sh.follow.store(true, Release);
        decoder.sh.refused().push(96_000);
        let mut st = state(Vec::new(), 0, None);
        // On a 44.1 kHz stream, a 96 kHz track needs the engine's 48 kHz stream.
        st.stream_rate = 44_100;
        assert_eq!(decoder.reopen_target(&st, 96_000), Some(48_000));
        // On the 48 kHz stream it is resampled in place: no new stream.
        st.stream_rate = 48_000;
        assert_eq!(decoder.reopen_target(&st, 96_000), None);
        // A rate nobody refused is still followed.
        assert_eq!(decoder.reopen_target(&st, 44_100), Some(44_100));
    }

    /// A moment `ms` after the `Instant` and the injected-clock time the throttle test starts at.
    fn at(base: (Instant, UtcMillis), monotonic_ms: u64, injected_ms: i64) -> (Instant, UtcMillis) {
        (
            base.0 + Duration::from_millis(monotonic_ms),
            UtcMillis::new(base.1.as_i64() + injected_ms),
        )
    }

    #[test]
    fn a_rate_flush_waits_out_the_interval_on_the_monotonic_clock() {
        let last = (Instant::now(), UtcMillis::new(1_000_000));
        // The injected clock stands still (a test's manual clock): the monotonic one decides.
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 0, 0)),
            RATE_FLUSH_INTERVAL
        );
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 100, 0)),
            Duration::from_millis(25)
        );
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 125, 0)),
            Duration::ZERO
        );
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 9_000, 0)),
            Duration::ZERO
        );
    }

    #[test]
    fn a_rate_flush_waits_out_the_interval_on_the_injected_clock_too() {
        let last = (Instant::now(), UtcMillis::new(1_000_000));
        // Real time has hardly passed, the injected clock was moved by hand.
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 1, 100)),
            Duration::from_millis(25)
        );
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 1, 125)),
            Duration::ZERO
        );
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 1, 130)),
            Duration::ZERO
        );
    }

    #[test]
    fn a_clock_set_back_never_holds_a_rate_change_up() {
        let last = (Instant::now(), UtcMillis::new(1_000_000));
        // The wall clock went back an hour: it says nothing, the monotonic clock decides.
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 100, -3_600_000)),
            Duration::from_millis(25)
        );
        assert_eq!(
            Decoder::throttle_wait(last, at(last, 125, -3_600_000)),
            Duration::ZERO
        );
        // Extremes do not overflow.
        let far = (last.0, UtcMillis::new(i64::MAX));
        let back = (
            last.0 + Duration::from_millis(200),
            UtcMillis::new(i64::MIN),
        );
        assert_eq!(Decoder::throttle_wait(far, back), Duration::ZERO);
    }

    #[test]
    fn an_item_is_reported_failed_once_and_the_heard_one_leaves_failed() {
        let events = Arc::new(Events::default());
        let (_tx, rx) = std::sync::mpsc::channel();
        let (producer, _consumer) = rtrb::RingBuffer::new(8);
        let mut decoder = Decoder {
            sh: Shared::new(1.0),
            rx,
            producer,
            opener: Arc::new(crate::FsOpener),
            sink: events.clone(),
            clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
            sample_rate: 48_000,
        };
        let mut st = state(vec![track(1), track(2)], 0, None);
        let error = || PlaybackError::Decode("again".into());
        decoder.track_failed(&mut st, QueueItemId(1), error());
        decoder.track_failed(&mut st, QueueItemId(1), error());
        decoder.track_failed(&mut st, QueueItemId(2), error());
        let seen = || {
            events
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        };
        assert_eq!(seen(), 2, "item 1 once, item 2 once");
        assert!(!st.item_failed, "nothing was being reopened");

        // The item a rate restart reopens: still one report, and it leaves as failed.
        st.heard = Some(QueueItemId(2));
        decoder.track_failed(&mut st, QueueItemId(1), error());
        assert!(!st.item_failed, "another item failing does not fail it");
        decoder.track_failed(&mut st, QueueItemId(2), error());
        assert!(st.item_failed);
        assert_eq!(st.heard, None);
        assert_eq!(seen(), 2, "no second report");
        st.reported.clear();
        decoder.track_failed(&mut st, QueueItemId(2), error());
        assert_eq!(seen(), 3, "a command's restart reports afresh");
    }

    #[test]
    fn a_rate_flush_is_polled_at_the_minimum_and_a_commands_backs_off() {
        let (tx, rx) = std::sync::mpsc::channel();
        // An acknowledgement that does not come: the first poll interval, then each next one.
        let mut audible = FLUSH_WAIT_MIN;
        let mut command = FLUSH_WAIT_MIN;
        let mut intervals = Vec::new();
        for _ in 0..8 {
            assert!(Decoder::wait_for_ack(&rx, &Restart::Audible, &mut audible)
                .expect("connected")
                .is_none());
            assert!(
                Decoder::wait_for_ack(&rx, &Restart::At { frame: 0, gen: 1 }, &mut command)
                    .expect("connected")
                    .is_none()
            );
            intervals.push(command);
        }
        assert_eq!(audible, FLUSH_WAIT_MIN, "a rate flush never backs off");
        // A command's flush does, up to the maximum (a stream that is not running).
        assert_eq!(
            intervals[..4],
            [
                Duration::from_millis(2),
                Duration::from_millis(4),
                Duration::from_millis(8),
                Duration::from_millis(16)
            ]
        );
        assert_eq!(command, FLUSH_WAIT_MAX);
        // A command arriving starts the backoff over; the control side leaving ends the thread.
        tx.send(Cmd::RateTarget).expect("send");
        let got = Decoder::wait_for_ack(&rx, &Restart::At { frame: 0, gen: 1 }, &mut command);
        assert!(matches!(got, Ok(Some(Cmd::RateTarget))));
        assert_eq!(command, FLUSH_WAIT_MIN);
        drop(tx);
        assert!(Decoder::wait_for_ack(&rx, &Restart::Audible, &mut audible).is_err());
    }

    /// A resampler that fails while flushing cannot be built (it only fails on a buffer-size bug),
    /// so this checks the reporting that follows such a failure: it names the track whose tail was
    /// lost, the last one opened, not whatever precedes `cur`.
    #[test]
    fn a_failed_flush_blames_the_last_track_that_was_read() {
        let events = Arc::new(Events::default());
        let (_tx, rx) = std::sync::mpsc::channel();
        let (producer, _consumer) = rtrb::RingBuffer::new(8);
        let mut decoder = Decoder {
            sh: Shared::new(1.0),
            rx,
            producer,
            opener: Arc::new(crate::FsOpener),
            sink: events.clone(),
            clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
            sample_rate: 48_000,
        };
        // Item 1 was the last one opened; items 2 and 3 failed to open, so `cur` is past the end.
        let mut st = state(vec![track(1), track(2), track(3)], 3, Some(0));
        decoder.flush_failed(&mut st, PlaybackError::Decode("resampler: test".into()));
        assert!(st.item_failed);
        let seen = events
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(
            seen,
            vec![EventKind::TrackFailed {
                item: QueueItemId(1),
                error: PlaybackError::Decode("resampler: test".into()),
            }]
        );
    }
}
