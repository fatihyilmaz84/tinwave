//! Exact track boundaries and positions with no current track: a track that has just finished is
//! never labelled failed, and no event position names a track nobody announced.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::*;
use tinwave_playback::{EventKind, LeaveCause, QueueItemId, StartCause};

fn wav_pair(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, TempDir) {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);
    (a, b, dir)
}

fn no_failure(kinds: &[EventKind]) {
    assert!(
        !kinds.iter().any(|k| matches!(
            k,
            EventKind::SeekFailed { .. }
                | EventKind::TrackFailed { .. }
                | EventKind::TrackLeft {
                    cause: LeaveCause::Failed,
                    ..
                }
        )),
        "{kinds:?}"
    );
}

fn finished_and_advanced(kinds: &[EventKind]) {
    assert!(
        kinds.contains(&EventKind::TrackLeft {
            item: QueueItemId(1),
            frame: SPLIT as u64,
            cause: LeaveCause::Finished
        }),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&EventKind::TrackStarted {
            item: QueueItemId(2),
            frame: 0,
            cause: StartCause::Advance
        }),
        "{kinds:?}"
    );
}

fn plays_b_from_its_start(h: &mut Harness) {
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, ref_r) = reference(&l64, &r64, 16);
    let rec = h.render_exact(4_800, 480);
    assert_eq!(max_err(&rec, &ref_l, &ref_r, SPLIT), 0.0);
}

/// A device switch whose resume point is exactly the end of the playing track (the sliver between
/// the last frame of A and the announcement of B) must treat A as finished and advance, not
/// reject the resume as an out-of-range seek and call A failed.
#[test]
fn switch_device_exactly_at_a_track_end_finishes_the_track_and_advances() {
    let (a, b, _dir) = wav_pair("boundary-switch");
    let gate = Arc::new(Gate::default());
    let opener = Arc::new(GatedOpener {
        gate: Arc::clone(&gate),
        allow: 0,
        only: Some(b.clone()),
    });
    let mut h = Harness::with_opener(opener);
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    // A's last frame is heard; the decoder is stuck opening B, so B has no mark yet.
    let _ = h.render_exact(SPLIT, 480);
    for _ in 0..30_000 {
        if gate.blocked.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    let at = h.engine.position().expect("position");
    assert_eq!((at.item.0, at.frame), (1, SPLIT as u64));

    h.engine
        .switch_device(Some("other".into()))
        .expect("switch");
    gate.open();
    h.settle();
    plays_b_from_its_start(&mut h);

    let kinds = h.sink.kinds();
    no_failure(&kinds);
    finished_and_advanced(&kinds);
    assert_paired(&kinds, true);
}

/// The same for a user seek to the very end (a seek bar dragged to 100 %), in each container.
#[test]
fn seeking_to_exactly_the_end_finishes_the_track_and_advances() {
    let dir = TempDir::new("boundary-seek-end");
    let (l, r) = sweep(RATE, TOTAL);
    let wav_a = dir.path("a.wav");
    let wav_b = dir.path("b.wav");
    write_wav(&wav_a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&wav_b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);
    let cases = [
        ("wav", wav_a, wav_b),
        (
            "flac",
            fixture("sweep_flac16_a.flac"),
            fixture("sweep_flac16_b.flac"),
        ),
        (
            "alac",
            fixture("sweep_alac_a.m4a"),
            fixture("sweep_alac_b.m4a"),
        ),
    ];
    for (name, a, b) in cases {
        let mut h = Harness::new();
        h.engine
            .load(vec![track(1, &a), track(2, &b)], 0, 0)
            .expect("load");
        let _ = h.render_exact(480, 480);
        h.engine.seek(SPLIT as u64).expect("seek to the end");
        h.settle();
        plays_b_from_its_start(&mut h);
        let kinds = h.sink.kinds();
        no_failure(&kinds);
        finished_and_advanced(&kinds);
        assert!(
            kinds.contains(&EventKind::Seeked {
                item: QueueItemId(1),
                from_frame: 480,
                to_frame: SPLIT as u64
            }),
            "{name}: {kinds:?}"
        );
        assert_paired(&kinds, true);
    }
}

/// Seeking to the end of the last track ends the queue, as finished.
#[test]
fn seeking_to_the_end_of_the_last_track_ends_the_queue_as_finished() {
    let (a, _b, _dir) = wav_pair("boundary-seek-last");
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    let _ = h.render_exact(480, 480);
    h.engine.seek(SPLIT as u64).expect("seek to the end");
    h.settle();
    h.run_until("queue ended", |h| h.saw(|k| *k == EventKind::QueueEnded));
    let kinds = h.sink.kinds();
    no_failure(&kinds);
    assert!(kinds.contains(&EventKind::TrackLeft {
        item: QueueItemId(1),
        frame: SPLIT as u64,
        cause: LeaveCause::Finished
    }));
    assert_paired(&kinds, false);
}

/// One frame past the end is still an error.
#[test]
fn seeking_past_the_end_is_still_a_failure() {
    let (a, b, _dir) = wav_pair("boundary-past-end");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(480, 480);
    h.engine.seek(SPLIT as u64 + 1).expect("accepted");
    h.settle();
    let kinds = h.sink.kinds();
    assert!(
        kinds.iter().any(|k| matches!(
            k,
            EventKind::SeekFailed {
                item: QueueItemId(1),
                ..
            }
        )),
        "{kinds:?}"
    );
}

/// With no current track the position is `None`, even while a track appended to the ended queue
/// is already decoded and its mark is in place; so `Paused`, `VolumeChanged` and `Resumed` carry
/// no item that never had a `TrackStarted`.
#[test]
fn no_event_position_names_an_unannounced_track() {
    let (a, b, _dir) = wav_pair("boundary-unannounced");
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    let _ = h.render_exact(SPLIT, 480);
    h.finish();
    assert_eq!(h.engine.position(), None, "the queue ended");

    // Append: B is decoded and its mark starts exactly where A ended, but nothing is heard yet
    // and no poll has announced it.
    h.engine.append(vec![track(2, &b)]).expect("append");
    h.wait_buffered(1);
    assert_eq!(h.engine.position(), None, "B is not announced yet");

    h.sink.take();
    h.engine.pause();
    h.engine.set_volume(0.5).expect("volume");
    h.engine.resume();
    let kinds = h.sink.take();
    assert_eq!(kinds.len(), 3, "{kinds:?}");
    assert!(
        matches!(kinds[0], EventKind::Paused { position: None }),
        "{kinds:?}"
    );
    assert!(
        matches!(kinds[1], EventKind::VolumeChanged { position: None, .. }),
        "{kinds:?}"
    );
    assert!(
        matches!(kinds[2], EventKind::Resumed { position: None }),
        "{kinds:?}"
    );

    // Once B is heard and announced, positions name it.
    h.run_until("B announced", |h| {
        h.saw(|k| {
            matches!(
                k,
                EventKind::TrackStarted {
                    item: QueueItemId(2),
                    ..
                }
            )
        })
    });
    assert_eq!(h.engine.position().map(|p| p.item), Some(QueueItemId(2)));
}

/// A track that failed to open is announced by the command but never played: until a later
/// track's first frames are heard and announced, the position is the command's target, not that
/// later track.
#[test]
fn a_failed_first_choice_does_not_leak_the_next_tracks_position() {
    let (a, _b, dir) = wav_pair("boundary-failed-open");
    let garbage = dir.path("garbage.wav");
    std::fs::write(&garbage, vec![0x55u8; 2_048]).expect("write");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &garbage), track(2, &a)], 0, 0)
        .expect("load");
    h.settle();
    // Item 2's mark exists now, but nothing has announced it.
    let p = h.engine.position().expect("the command's target");
    assert_eq!((p.item.0, p.frame), (1, 0));
    h.run_until("item 2 announced", |h| {
        h.saw(|k| {
            matches!(
                k,
                EventKind::TrackStarted {
                    item: QueueItemId(2),
                    ..
                }
            )
        })
    });
    assert_eq!(h.engine.position().map(|p| p.item), Some(QueueItemId(2)));
    assert_paired(&h.sink.kinds(), true);
}
