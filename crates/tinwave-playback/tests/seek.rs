//! Seek test: after a seek the output equals the reference sweep from exactly the requested frame.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use common::*;
use tinwave_playback::{EventKind, QueueItemId};

struct Case {
    name: &'static str,
    a: std::path::PathBuf,
    b: std::path::PathBuf,
    /// `Some(bits)` = lossless, must reproduce the quantised reference exactly; `None` = lossy.
    bits: Option<u32>,
    lossy_tolerance: f32,
}

fn cases() -> Vec<(Case, Option<TempDir>)> {
    let dir = TempDir::new("seek-wav16");
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);
    vec![
        (
            Case {
                name: "wav16",
                a,
                b,
                bits: Some(16),
                lossy_tolerance: 0.0,
            },
            Some(dir),
        ),
        (
            Case {
                name: "flac16",
                a: fixture("sweep_flac16_a.flac"),
                b: fixture("sweep_flac16_b.flac"),
                bits: Some(16),
                lossy_tolerance: 0.0,
            },
            None,
        ),
        (
            Case {
                name: "alac16",
                a: fixture("sweep_alac_a.m4a"),
                b: fixture("sweep_alac_b.m4a"),
                bits: Some(16),
                lossy_tolerance: 0.0,
            },
            None,
        ),
        (
            Case {
                name: "mp3lame",
                a: fixture("sweep_mp3lame_a.mp3"),
                b: fixture("sweep_mp3lame_b.mp3"),
                bits: None,
                lossy_tolerance: 0.02,
            },
            None,
        ),
    ]
}

/// Seek to `frame` of item `item` (1 = A, 2 = B), record `length` frames, compare.
fn seek_and_check(
    h: &mut Harness,
    c: &Case,
    item: u64,
    frame: u64,
    length: usize,
    refs: &(Vec<f32>, Vec<f32>),
) {
    // Items are 1 (A) and 2 (B); seeking in B first needs B to be current, which is `load`.
    let base = if item == 2 { SPLIT } else { 0 };
    if item == 2 {
        h.engine
            .load(vec![track(1, &c.a), track(2, &c.b)], 1, frame)
            .expect("load B");
    } else {
        h.engine.seek(frame).expect("seek");
    }
    h.settle();
    let rec = h.render_exact(length, 333);
    let start = base + frame as usize;
    let err = max_err(&rec, &refs.0, &refs.1, start);
    match c.bits {
        Some(_) => assert_eq!(
            err, 0.0,
            "{}: seek to {item}:{frame} must be bit-exact, off by {err}",
            c.name
        ),
        None => {
            let (rec_l, _) = deinterleave(&rec);
            let lag = best_lag(&rec_l, &refs.0[start..], 1_000, 500, 4);
            assert_eq!(
                lag, 0,
                "{}: seek to {item}:{frame} landed {lag} frames off",
                c.name
            );
            assert!(err <= c.lossy_tolerance, "{}: seek error {err}", c.name);
        }
    }
}

#[test]
fn seek_lands_sample_exact() {
    let (l64, r64) = sweep(RATE, TOTAL);
    for (case, _keep_dir) in cases() {
        let refs = reference(&l64, &r64, case.bits.unwrap_or(0));
        let mut h = Harness::new();
        h.latency = 480;
        h.engine
            .load(vec![track(1, &case.a), track(2, &case.b)], 0, 0)
            .expect("load");
        // Listen for a while first so the ring is full and the seek has data to discard.
        let _ = h.render_exact(7_000, 480);

        // Odd frames, not codec-frame aligned; backwards and forwards; start of track; near the
        // end of A: the last one plays across the hand-off into B, so it must come last.
        for frame in [
            12_345u64,
            1,
            40_001,
            3,
            47_000,
            20_000,
            (SPLIT - 600) as u64,
        ] {
            seek_and_check(&mut h, &case, 1, frame, 1_500, &refs);
        }
        // Into B (a `load` at an offset), then a seek inside B.
        seek_and_check(&mut h, &case, 2, 5_001, 1_500, &refs);
        seek_and_check(&mut h, &case, 2, 30_000, 1_500, &refs);
    }
}

#[test]
fn seek_reports_exact_from_and_to_frames() {
    let (a, b) = (
        fixture("sweep_flac16_a.flac"),
        fixture("sweep_flac16_b.flac"),
    );
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(4_800, 480);
    let before = h.engine.position().expect("position");
    assert_eq!(before.item, QueueItemId(1));
    assert_eq!(before.frame, 4_800);

    h.sink.take();
    h.clock.advance(250);
    h.engine.seek(30_001).expect("seek");
    // The command's target is reported immediately, before the decoder has caught up.
    let pending = h.engine.position().expect("position");
    assert_eq!((pending.item, pending.frame), (QueueItemId(1), 30_001));

    let events = h.sink.all();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].at.as_i64(),
        1_000_250,
        "events carry the injected clock"
    );
    assert_eq!(
        events[0].kind,
        EventKind::Seeked {
            item: QueueItemId(1),
            from_frame: 4_800,
            to_frame: 30_001
        }
    );
    h.settle();
    let after = h.engine.position().expect("position");
    assert_eq!(after.frame, 30_001);
}

#[test]
fn seek_past_the_end_is_reported_and_the_queue_moves_on() {
    let (a, b) = (
        fixture("sweep_flac16_a.flac"),
        fixture("sweep_flac16_b.flac"),
    );
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(480, 480);
    h.sink.take();
    h.engine.seek(10_000_000).expect("seek is accepted");
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
    // B plays from its start.
    let rec = h.render_exact(1_000, 500);
    let (l64, r64) = sweep(RATE, TOTAL);
    let refs = reference(&l64, &r64, 16);
    assert_eq!(max_err(&rec, &refs.0, &refs.1, SPLIT), 0.0);
}
