//! The decoder thread: reads tracks with Symphonia and keeps the ring full.
//!
//! This is the only thread that touches files. It never waits on the render callback except at the
//! seek handshake, and even there it keeps servicing commands.

use std::collections::HashSet;
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use rtrb::Producer;
use tinwave_core::Clock;

use crate::event::{EventKind, EventSink, PlaybackEvent};
use crate::reader::{SourceOpener, TrackReader};
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
        /// `None` when nothing is loaded.
        item: Option<QueueItemId>,
        frame: u64,
        gen: u64,
    },
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
            Self::Shutdown => "Shutdown",
        })
    }
}

/// Idle poll interval while the ring is full. The ring holds one second, so this keeps it at
/// least ~97 % full while costing about 40 wakeups per second.
const FULL_RING_WAIT: Duration = Duration::from_millis(25);
/// Poll interval while waiting for the renderer to acknowledge a flush: starts short (a running
/// stream acks within a callback) and backs off, so a stream that is not running (failed device
/// switch) costs about 20 wakeups per second instead of 1000.
const FLUSH_WAIT_MIN: Duration = Duration::from_millis(1);
const FLUSH_WAIT_MAX: Duration = Duration::from_millis(50);

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
    reader: Option<TrackReader>,
    pending: Vec<f32>,
    pending_off: usize,
    /// Stream timeline: renderer-consumed frames at the last flush plus frames pushed since.
    written: u64,
    next_seq: u64,
    mark_kind: MarkKind,
    /// The item that started last failed mid-decode (copied into the next mark and the end of
    /// stream, so the control side can tell `Failed` from `Finished`).
    item_failed: bool,
    /// A flush was requested; once the renderer acks `req`, restart at `cur`/`frame`.
    awaiting: Option<Awaiting>,
}

struct Awaiting {
    req: u64,
    frame: u64,
    gen: u64,
}

impl Decoder {
    pub fn run(mut self) {
        let mut st = State {
            queue: Vec::new(),
            cur: 0,
            reader: None,
            pending: Vec::new(),
            pending_off: 0,
            written: 0,
            next_seq: 0,
            mark_kind: MarkKind::Initial,
            item_failed: false,
            awaiting: None,
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

            if let Some(awaiting) = &st.awaiting {
                if self.sh.flush_ack.load(Acquire) != awaiting.req {
                    match self.rx.recv_timeout(flush_wait) {
                        Ok(cmd) => {
                            flush_wait = FLUSH_WAIT_MIN;
                            if !self.accept(&mut st, cmd) {
                                return;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            flush_wait = (flush_wait * 2).min(FLUSH_WAIT_MAX);
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    continue;
                }
                let (frame, gen) = (awaiting.frame, awaiting.gen);
                flush_wait = FLUSH_WAIT_MIN;
                st.awaiting = None;
                self.restart(&mut st, frame, gen);
            }

            if st.pending_off >= st.pending.len() {
                st.pending.clear();
                st.pending_off = 0;
                if !self.fill_pending(&mut st) {
                    // End of queue: sleep until a command arrives.
                    match self.rx.recv() {
                        Ok(cmd) => {
                            if !self.accept(&mut st, cmd) {
                                return;
                            }
                        }
                        Err(_) => return,
                    }
                    continue;
                }
            }

            // Whole frames only, so the stream timeline never splits a frame.
            let free = self.producer.slots() & !1;
            let take = free.min(st.pending.len() - st.pending_off);
            if take == 0 {
                match self.rx.recv_timeout(FULL_RING_WAIT) {
                    Ok(cmd) => {
                        if !self.accept(&mut st, cmd) {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
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
            Cmd::Load {
                queue,
                index,
                frame,
                gen,
            } => {
                self.drop_reader(st);
                st.queue = queue;
                st.cur = index;
                self.begin_flush(st, frame, gen);
            }
            Cmd::Seek { item, frame, gen } => match st.queue.iter().position(|t| t.id == item) {
                Some(index) => {
                    self.drop_reader(st);
                    st.cur = index;
                    self.begin_flush(st, frame, gen);
                }
                None => {
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
                item,
                frame,
                gen,
            } => {
                self.producer = producer;
                self.drop_reader(st);
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
        }
        true
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
                st.queue.extend(items);
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
        // read; once the queue is exhausted the last item started is the one before `cur`.
        let started = if st.reader.is_some() || st.awaiting.is_some() {
            Some(st.cur)
        } else {
            st.cur.checked_sub(1)
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

    fn begin_flush(&mut self, st: &mut State, frame: u64, gen: u64) {
        debug_assert!(
            st.reader.is_none(),
            "readers are dropped through drop_reader"
        );
        st.pending.clear();
        st.pending_off = 0;
        let req = self.sh.flush_req.fetch_add(1, AcqRel) + 1;
        st.awaiting = Some(Awaiting { req, frame, gen });
    }

    /// Start a fresh stream timeline at the renderer's current position and open `st.cur`.
    fn restart(&mut self, st: &mut State, frame: u64, gen: u64) {
        self.reset_stream(st);
        st.mark_kind = MarkKind::Initial;
        if st.cur >= st.queue.len() || !self.open_current(st, frame) {
            self.finish_queue(st);
        }
        // Published last: the control thread then sees the first mark (or the end of stream).
        self.sh.applied_gen.fetch_max(gen, Release);
    }

    fn reset_stream(&mut self, st: &mut State) {
        debug_assert!(
            st.reader.is_none(),
            "readers are dropped through drop_reader"
        );
        st.item_failed = false;
        st.pending.clear();
        st.pending_off = 0;
        st.written = self.sh.consumed.load(Acquire);
        self.sh.marks().clear();
        self.sh.buffering.store(false, Release);
        self.sh.eos.store(false, Release);
    }

    fn finish_queue(&mut self, st: &mut State) {
        self.sh.eos_total.store(st.written, Release);
        self.sh.eos_failed.store(st.item_failed, Release);
        self.sh.buffering.store(false, Release);
        self.sh.eos.store(true, Release);
    }

    /// Open `st.cur` at `frame`, moving on to later queue entries when one fails.
    /// Returns false when the queue is exhausted.
    fn open_current(&mut self, st: &mut State, mut frame: u64) -> bool {
        while let Some(track) = st.queue.get(st.cur).cloned() {
            let opened = TrackReader::open(self.opener.as_ref(), &track).and_then(|reader| {
                if reader.sample_rate() == self.sample_rate {
                    Ok(reader)
                } else {
                    Err(PlaybackError::SampleRateMismatch {
                        track_rate: reader.sample_rate(),
                        output_rate: self.sample_rate,
                    })
                }
            });
            match opened {
                Ok(mut reader) => {
                    if frame > 0 {
                        if let Err(error) = reader.seek(frame) {
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
                    let seq = st.next_seq;
                    st.next_seq += 1;
                    self.sh.marks().push_back(Mark {
                        seq,
                        start_frame: st.written,
                        item: track,
                        offset: frame,
                        kind: st.mark_kind,
                        prev_failed: st.item_failed,
                    });
                    st.item_failed = false;
                    st.mark_kind = MarkKind::Continuation;
                    st.reader = Some(reader);
                    return true;
                }
                Err(error) => {
                    self.emit(EventKind::TrackFailed {
                        item: track.id,
                        error,
                    });
                    st.cur += 1;
                    frame = 0;
                }
            }
        }
        false
    }

    /// Decode more audio into `st.pending`, advancing through the queue. `false` at the end.
    fn fill_pending(&mut self, st: &mut State) -> bool {
        loop {
            if st.reader.is_none() && !self.open_current(st, 0) {
                self.finish_queue(st);
                return false;
            }
            let Some(reader) = st.reader.as_mut() else {
                continue;
            };
            let result = reader.next_chunk(&mut st.pending);
            match result {
                Ok(true) => return true,
                Ok(false) => self.finish_item(st),
                Err(error) => {
                    if let Some(track) = st.queue.get(st.cur) {
                        self.emit(EventKind::TrackFailed {
                            item: track.id,
                            error,
                        });
                    }
                    st.item_failed = true;
                    self.finish_item(st);
                }
            }
        }
    }

    fn finish_item(&mut self, st: &mut State) {
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

    fn emit(&self, kind: EventKind) {
        self.sink.emit(PlaybackEvent {
            at: self.clock.now(),
            kind,
        });
    }
}
