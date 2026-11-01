//! Position marks (10 Hz of heard audio, monotonic) and track-boundary events.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use common::*;
use tinwave_playback::{EventKind, LeaveCause, QueueItemId, StartCause};

fn pair() -> (std::path::PathBuf, std::path::PathBuf) {
    (
        fixture("sweep_flac16_a.flac"),
        fixture("sweep_flac16_b.flac"),
    )
}

#[test]
fn position_marks_are_monotonic_and_ten_per_second_of_audio() {
    let (a, b) = pair();
    let mut h = Harness::new();
    h.latency = 960; // 20 ms between callback and speaker
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");

    // Poll after every 480-frame (10 ms) callback, like a UI timer faster than 10 Hz.
    let rec = h.render_exact(TOTAL, 480);
    assert_eq!(rec.len(), TOTAL * 2);
    h.finish();

    let marks: Vec<(u64, u64, u64)> = h
        .sink
        .kinds()
        .into_iter()
        .filter_map(|k| match k {
            EventKind::PositionMark { position } => {
                Some((position.item.0, position.frame, position.position_ms))
            }
            _ => None,
        })
        .collect();

    // 2 s of audio => about 20 marks (the last 20 ms of latency never gets "heard" before the end).
    assert!((18..=20).contains(&marks.len()), "{} marks", marks.len());
    for pair in marks.windows(2) {
        // (item, frame) strictly increases lexicographically across the whole run.
        assert!(
            (pair[1].0, pair[1].1) > (pair[0].0, pair[0].1),
            "position went backwards: {pair:?}"
        );
        if pair[0].0 == pair[1].0 {
            // Within one track: exactly 100 ms of audio apart (4800 frames), because polling is
            // faster than the mark rate and marks follow heard audio, not wall time.
            assert_eq!(pair[1].1 - pair[0].1, 4_800, "{pair:?}");
        }
    }
    // The first mark is the first 100 ms of heard audio (frame 4800), reported once the callback
    // has taken 4800 frames plus the 960 latency frames.
    assert_eq!(marks[0].0, 1);
    assert_eq!(marks[0].1, 4_800);
    assert_eq!(marks[0].2, 100);
}

#[test]
fn a_late_poll_yields_one_mark_not_a_burst() {
    let (a, b) = pair();
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    h.settle();
    h.sink.take();
    // Render 1 s without polling in between.
    for _ in 0..100 {
        h.wait_buffered(480);
        let mut block = vec![0.0f32; 960];
        h.out.render(&mut block, 0).expect("open");
    }
    h.engine.poll();
    let marks = h
        .sink
        .kinds()
        .into_iter()
        .filter(|k| matches!(k, EventKind::PositionMark { .. }))
        .count();
    assert_eq!(marks, 1);
}

#[test]
fn track_boundaries_and_end_of_queue_are_reported_with_exact_frames() {
    let (a, b) = pair();
    let mut h = Harness::new();
    h.latency = 480;
    h.clock.advance(5);
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(TOTAL, 512);
    h.finish();

    let kinds: Vec<EventKind> = h
        .sink
        .kinds()
        .into_iter()
        .filter(|k| {
            !matches!(
                k,
                EventKind::PositionMark { .. } | EventKind::Underrun { .. }
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::TrackStarted {
                item: QueueItemId(1),
                frame: 0,
                cause: StartCause::Requested
            },
            EventKind::TrackLeft {
                item: QueueItemId(1),
                frame: SPLIT as u64,
                cause: LeaveCause::Finished
            },
            EventKind::TrackStarted {
                item: QueueItemId(2),
                frame: 0,
                cause: StartCause::Advance
            },
            EventKind::TrackLeft {
                item: QueueItemId(2),
                frame: (TOTAL - SPLIT) as u64,
                cause: LeaveCause::Finished
            },
            EventKind::QueueEnded,
        ]
    );
    // Every event carries the injected clock (frozen at 1_000_005 here), never the system clock.
    assert!(h.sink.all().iter().all(|e| e.at.as_i64() == 1_000_005));
}

#[test]
fn load_while_playing_reports_the_skipped_track_position() {
    let (a, b) = pair();
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(9_600, 480);
    h.sink.take();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 1, 0)
        .expect("skip");
    assert_eq!(
        h.sink.take(),
        vec![
            EventKind::TrackLeft {
                item: QueueItemId(1),
                frame: 9_600,
                cause: LeaveCause::Replaced
            },
            EventKind::TrackStarted {
                item: QueueItemId(2),
                frame: 0,
                cause: StartCause::Requested
            },
        ]
    );
    h.settle();
    let _ = h.render_exact(4_800, 480);
    // Adopting the new stream emits nothing extra: the command already announced it.
    let extra: Vec<EventKind> = h
        .sink
        .take()
        .into_iter()
        .filter(|k| !matches!(k, EventKind::PositionMark { .. }))
        .collect();
    assert!(extra.is_empty(), "{extra:?}");
}

#[test]
fn replacing_a_track_that_was_never_heard_still_closes_it() {
    let (a, b) = pair();
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 1, 0)
        .expect("skip before anything played");
    assert_eq!(
        h.sink.take(),
        vec![
            EventKind::TrackStarted {
                item: QueueItemId(1),
                frame: 0,
                cause: StartCause::Requested
            },
            EventKind::TrackLeft {
                item: QueueItemId(1),
                frame: 0,
                cause: LeaveCause::Replaced
            },
            EventKind::TrackStarted {
                item: QueueItemId(2),
                frame: 0,
                cause: StartCause::Requested
            },
        ]
    );
}
