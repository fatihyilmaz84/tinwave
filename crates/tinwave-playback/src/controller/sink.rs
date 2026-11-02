//! The bounded queue between the engine and the controller (P-0042, `docs/rules/rust.md`
//! "Concurrency").
//!
//! The engine emits from the control thread and from the decoder thread, and must never wait for
//! its consumer. [`BoundedSink`] is the [`EventSink`] it is given: one short critical section to
//! append, no allocation after construction, never blocking beyond that lock.
//!
//! # Two classes, one ordered queue
//!
//! Events keep the order they were emitted in (the mapping to signals depends on it), so there is
//! one queue, with two budgets:
//!
//! - **Position marks** (10 Hz while playing) are the only high-frequency event, and within one
//!   track only the newest is useful for the position. But the *first* mark that names a track is
//!   what tells the controller that the track was heard (`PlayStarted`), so a mark is only
//!   coalesced away when the next mark names the same track. Marks may hold at most
//!   `mark_capacity` slots; when that is full the oldest mark that is followed by a mark of the
//!   same track is removed to make room (counted in [`SinkStats::coalesced_marks`]); only if every
//!   mark names a different track is the plain oldest removed. They can never take a slot from
//!   the other class.
//! - **Everything else** (`TrackStarted`, `TrackLeft`, `Paused`, `Seeked`, ...) has
//!   `critical_capacity` slots of its own and is never removed. If the controller stalls for so
//!   long that those fill up, the *newest* event is lost and counted in
//!   [`SinkStats::overflowed`]; that is a bug to fix (the controller is dead), not a policy, and
//!   the controller logs it at `error` and shows it in its status. The default capacity is far
//!   above what a 100 ms poll can produce (a handful of events per poll).

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::event::{EventKind, EventSink, PlaybackEvent};
use crate::QueueItemId;

/// Slots for events that are never dropped.
pub const CRITICAL_CAPACITY: usize = 1_024;
/// Slots for position marks.
pub const MARK_CAPACITY: usize = 8;

/// Losses a [`BoundedSink`] has had since it was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SinkStats {
    /// Position marks replaced by a newer one because the mark budget was full. Normal under load.
    pub coalesced_marks: u64,
    /// Non-mark events lost because the critical budget was full. Must stay 0.
    pub overflowed: u64,
}

#[derive(Debug)]
struct State {
    queue: VecDeque<PlaybackEvent>,
    marks: usize,
    critical: usize,
    stats: SinkStats,
}

/// The engine's event sink: bounded, ordered, with a reserved budget for events that must not be
/// lost. See the module docs.
#[derive(Debug)]
pub struct BoundedSink {
    state: Mutex<State>,
    critical_capacity: usize,
    mark_capacity: usize,
}

impl BoundedSink {
    /// A sink with the default capacities.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(CRITICAL_CAPACITY, MARK_CAPACITY)
    }

    /// A sink with explicit budgets (each at least 1).
    #[must_use]
    pub fn with_capacity(critical_capacity: usize, mark_capacity: usize) -> Self {
        let critical_capacity = critical_capacity.max(1);
        let mark_capacity = mark_capacity.max(1);
        Self {
            state: Mutex::new(State {
                queue: VecDeque::with_capacity(critical_capacity + mark_capacity),
                marks: 0,
                critical: 0,
                stats: SinkStats::default(),
            }),
            critical_capacity,
            mark_capacity,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock cannot leave the counters inconsistent with the queue
        // (every mutation is a single push or remove followed by its count), so keep going.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves every queued event, oldest first, to the end of `out`.
    pub fn drain_into(&self, out: &mut Vec<PlaybackEvent>) {
        let mut state = self.lock();
        out.extend(state.queue.drain(..));
        state.marks = 0;
        state.critical = 0;
    }

    /// Losses so far.
    #[must_use]
    pub fn stats(&self) -> SinkStats {
        self.lock().stats
    }
}

impl Default for BoundedSink {
    fn default() -> Self {
        Self::new()
    }
}

/// The mark to remove to make room for `incoming`: the oldest one that the next mark (or
/// `incoming`) names the same track as, so no track's first mark is lost; else the oldest mark.
fn coalescible_mark(queue: &VecDeque<PlaybackEvent>, incoming: &PlaybackEvent) -> Option<usize> {
    let item_of = |e: &PlaybackEvent| match &e.kind {
        EventKind::PositionMark { position } => Some(position.item),
        _ => None,
    };
    let mut oldest = None;
    let mut previous: Option<(usize, QueueItemId)> = None;
    for (index, item) in queue
        .iter()
        .enumerate()
        .filter_map(|(i, e)| item_of(e).map(|item| (i, item)))
    {
        oldest.get_or_insert(index);
        if let Some((before, before_item)) = previous {
            if before_item == item {
                return Some(before);
            }
        }
        previous = Some((index, item));
    }
    match (previous, item_of(incoming)) {
        (Some((before, before_item)), Some(item)) if before_item == item => Some(before),
        _ => oldest,
    }
}

impl EventSink for BoundedSink {
    fn emit(&self, event: PlaybackEvent) {
        let is_mark = matches!(event.kind, EventKind::PositionMark { .. });
        let mut state = self.lock();
        if is_mark {
            if state.marks >= self.mark_capacity {
                if let Some(index) = coalescible_mark(&state.queue, &event) {
                    state.queue.remove(index);
                    state.marks -= 1;
                    state.stats.coalesced_marks += 1;
                }
            }
            state.queue.push_back(event);
            state.marks += 1;
        } else if state.critical >= self.critical_capacity {
            state.stats.overflowed += 1;
        } else {
            state.queue.push_back(event);
            state.critical += 1;
        }
    }
}
