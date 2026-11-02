//! The bounded sink between the engine and the controller: a flood of position marks can never
//! push out the events that pair starts with leaves (P-0042).
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

use std::sync::Arc;

use tinwave_core::UtcMillis;
use tinwave_playback::{
    BoundedSink, EventKind, EventSink, LeaveCause, PlaybackEvent, Position, QueueItemId,
    StartCause, MARK_CAPACITY,
};

fn at(n: i64) -> UtcMillis {
    UtcMillis::new(n)
}

fn mark(n: u64) -> PlaybackEvent {
    mark_of(1, n)
}

fn mark_of(item: u64, n: u64) -> PlaybackEvent {
    PlaybackEvent {
        at: at(n as i64),
        kind: EventKind::PositionMark {
            position: Position {
                item: QueueItemId(item),
                frame: n,
                position_ms: n,
            },
        },
    }
}

fn started(item: u64) -> PlaybackEvent {
    PlaybackEvent {
        at: at(item as i64),
        kind: EventKind::TrackStarted {
            item: QueueItemId(item),
            frame: 0,
            cause: StartCause::Advance,
        },
    }
}

fn left(item: u64) -> PlaybackEvent {
    PlaybackEvent {
        at: at(item as i64),
        kind: EventKind::TrackLeft {
            item: QueueItemId(item),
            frame: 10,
            cause: LeaveCause::Finished,
        },
    }
}

fn drained(sink: &BoundedSink) -> Vec<PlaybackEvent> {
    let mut out = Vec::new();
    sink.drain_into(&mut out);
    out
}

fn criticals(events: &[PlaybackEvent]) -> Vec<EventKind> {
    events
        .iter()
        .filter(|e| !matches!(e.kind, EventKind::PositionMark { .. }))
        .map(|e| e.kind.clone())
        .collect()
}

#[test]
fn a_flood_of_position_marks_never_drops_a_start_or_a_leave() {
    let sink = BoundedSink::new();
    let mut expected = Vec::new();
    let mut n = 0;
    for track in 1..=50_u64 {
        let s = started(track);
        expected.push(s.kind.clone());
        sink.emit(s);
        for _ in 0..5_000 {
            n += 1;
            sink.emit(mark(n));
        }
        let l = left(track);
        expected.push(l.kind.clone());
        sink.emit(l);
    }
    let stats = sink.stats();
    assert_eq!(stats.overflowed, 0);
    assert!(stats.coalesced_marks > 200_000, "{stats:?}");
    let events = drained(&sink);
    assert_eq!(criticals(&events), expected, "all of them, in order");
    assert!(
        events.len() - expected.len() <= MARK_CAPACITY,
        "marks are bounded: {} left",
        events.len() - expected.len()
    );
}

#[test]
fn marks_are_coalesced_to_the_newest_and_keep_their_place_in_the_order() {
    let sink = BoundedSink::with_capacity(16, 3);
    sink.emit(started(1));
    for n in 1..=10 {
        sink.emit(mark(n));
    }
    sink.emit(left(1));
    sink.emit(mark(11));
    let events = drained(&sink);
    let shape: Vec<String> = events
        .iter()
        .map(|e| match &e.kind {
            EventKind::TrackStarted { .. } => "start".to_string(),
            EventKind::TrackLeft { .. } => "leave".to_string(),
            EventKind::PositionMark { position } => format!("mark{}", position.frame),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    // Marks 1..=10 leave 8, 9, 10; mark 11 then replaces the oldest (8), so the survivors are
    // 9 and 10 (before the leave) and 11 (after it): the order of emission is kept.
    assert_eq!(shape, ["start", "mark9", "mark10", "leave", "mark11"]);
}

#[test]
fn the_first_mark_of_a_track_is_never_coalesced_away_by_marks_of_another_track() {
    let sink = BoundedSink::with_capacity(16, 2);
    sink.emit(started(1));
    sink.emit(mark_of(1, 1));
    sink.emit(started(2));
    for n in 2..=50 {
        sink.emit(mark_of(2, n));
    }
    let frames: Vec<(u64, u64)> = drained(&sink)
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::PositionMark { position } => Some((position.item.0, position.frame)),
            _ => None,
        })
        .collect();
    assert_eq!(
        frames,
        [(1, 1), (2, 50)],
        "track 1 keeps its only mark; track 2 keeps its newest"
    );
}

#[test]
fn critical_events_have_their_own_budget_that_marks_cannot_use() {
    let sink = BoundedSink::with_capacity(4, 2);
    for n in 0..100 {
        sink.emit(mark(n));
    }
    for item in 1..=4 {
        sink.emit(started(item));
    }
    assert_eq!(sink.stats().overflowed, 0, "four slots were still free");
    let events = drained(&sink);
    assert_eq!(criticals(&events).len(), 4);
}

#[test]
fn when_the_critical_budget_is_exhausted_the_newest_is_lost_and_counted() {
    let sink = BoundedSink::with_capacity(4, 2);
    for item in 1..=10 {
        sink.emit(started(item));
    }
    sink.emit(mark(1));
    let stats = sink.stats();
    assert_eq!(stats.overflowed, 6);
    let events = drained(&sink);
    let kept: Vec<_> = criticals(&events);
    assert_eq!(kept.len(), 4);
    assert!(matches!(
        kept[0],
        EventKind::TrackStarted {
            item: QueueItemId(1),
            ..
        }
    ));
    // After the controller has caught up there is room again.
    sink.emit(started(11));
    assert_eq!(criticals(&drained(&sink)).len(), 1);
}

#[test]
fn two_producer_threads_and_a_slow_consumer_lose_no_critical_event() {
    let sink = Arc::new(BoundedSink::new());
    let flood = {
        let sink = Arc::clone(&sink);
        std::thread::spawn(move || {
            for n in 0..300_000 {
                sink.emit(mark(n));
            }
        })
    };
    let producer = {
        let sink = Arc::clone(&sink);
        std::thread::spawn(move || {
            for item in 1..=400_u64 {
                sink.emit(started(item));
                sink.emit(left(item));
                std::thread::yield_now();
            }
        })
    };
    let mut received = Vec::new();
    let mut scratch = Vec::new();
    while !(flood.is_finished() && producer.is_finished()) {
        sink.drain_into(&mut scratch);
        received.append(&mut scratch);
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    flood.join().expect("flood thread");
    producer.join().expect("producer thread");
    sink.drain_into(&mut scratch);
    received.append(&mut scratch);

    assert_eq!(sink.stats().overflowed, 0);
    let got = criticals(&received);
    let expected: Vec<_> = (1..=400_u64)
        .flat_map(|item| [started(item).kind, left(item).kind])
        .collect();
    assert_eq!(got, expected, "every start and leave, in order");
}
