//! Gapless test (ADR 0005, testing.md): a -20 dBFS sweep split into two files at a frame that is
//! not aligned to any codec frame must play across the boundary with a 0-frame gap, per format.
//!
//! WAV pairs are written by the test itself (tiny writer in `common`, no dev-dependency); FLAC,
//! ALAC and MP3 (LAME with Xing/Info tag) pairs are the committed files in `fixtures/audio/`
//! (see its README for how they were made).
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use common::*;
use tinwave_playback::EventKind;

/// Plays the A|B pair and checks it against the reference sweep.
/// `exact` formats must reproduce the reference bit for bit; lossy ones must be lag 0 on both
/// sides of the boundary and stay inside the codec's noise.
fn check_pair(
    a: std::path::PathBuf,
    b: std::path::PathBuf,
    bits: Option<u32>,
    lossy_tolerance: f32,
) {
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, ref_r) = reference(&l64, &r64, bits.unwrap_or(0));

    let mut h = Harness::new();
    h.latency = 480;
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");

    // Exactly the original length must come out, with no underrun silence in between.
    let rec = h.render_exact(TOTAL, 512);
    h.finish();

    let err = max_err(&rec, &ref_l, &ref_r, 0);
    let (rec_l, _) = deinterleave(&rec);
    let lag_a = best_lag(&rec_l, &ref_l, SPLIT - 3_000, 1_500, 4);
    let lag_b = best_lag(&rec_l, &ref_l, SPLIT + 3_000, 1_500, 4);
    let lag_at_boundary = best_lag(&rec_l, &ref_l, SPLIT, 200, 4);
    assert_eq!(lag_a, 0, "head of the pair is misaligned");
    assert_eq!(
        lag_b, 0,
        "tail of the pair is misaligned: gap of {lag_b} frames"
    );
    assert_eq!(
        lag_at_boundary, 0,
        "misaligned at the boundary: {lag_at_boundary} frames"
    );
    match bits {
        Some(_) => assert_eq!(
            err, 0.0,
            "lossless format must be bit-exact, deviates by {err}"
        ),
        None => assert!(err <= lossy_tolerance, "lossy format deviates by {err}"),
    }

    // Track bookkeeping: exact boundary frame in the events.
    let kinds = h.sink.kinds();
    assert!(kinds.contains(&EventKind::TrackLeft {
        item: tinwave_playback::QueueItemId(1),
        frame: SPLIT as u64,
        cause: tinwave_playback::LeaveCause::Finished,
    }));
    assert!(kinds.contains(&EventKind::TrackLeft {
        item: tinwave_playback::QueueItemId(2),
        frame: (TOTAL - SPLIT) as u64,
        cause: tinwave_playback::LeaveCause::Finished,
    }));
}

fn split_wav(bits: u32, tag: &str) -> (std::path::PathBuf, std::path::PathBuf, TempDir) {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, bits, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, bits, &l[SPLIT..], &r[SPLIT..]);
    (a, b, dir)
}

#[test]
fn wav16_gapless() {
    let (a, b, _dir) = split_wav(16, "gapless-wav16");
    check_pair(a, b, Some(16), 0.0);
}

#[test]
fn wav24_gapless() {
    let (a, b, _dir) = split_wav(24, "gapless-wav24");
    check_pair(a, b, Some(24), 0.0);
}

#[test]
fn wav_float32_gapless() {
    let (a, b, _dir) = split_wav(0, "gapless-wavf32");
    check_pair(a, b, Some(0), 0.0);
}

#[test]
fn flac16_gapless() {
    check_pair(
        fixture("sweep_flac16_a.flac"),
        fixture("sweep_flac16_b.flac"),
        Some(16),
        0.0,
    );
}

#[test]
fn alac16_gapless() {
    check_pair(
        fixture("sweep_alac_a.m4a"),
        fixture("sweep_alac_b.m4a"),
        Some(16),
        0.0,
    );
}

#[test]
fn mp3_with_lame_tag_gapless() {
    // 192 kbps CBR: the spike measured a worst-case sample error of 1.06e-2 against the sweep.
    check_pair(
        fixture("sweep_mp3lame_a.mp3"),
        fixture("sweep_mp3lame_b.mp3"),
        None,
        0.02,
    );
}

/// The detector itself: a recording with one frame missing or one extra frame at the boundary
/// must not read as lag 0 (otherwise the tests above would pass vacuously).
#[test]
fn lag_detector_sees_a_one_frame_gap() {
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, _) = reference(&l64, &r64, 16);

    let mut dropped: Vec<f32> = ref_l[..SPLIT].to_vec();
    dropped.extend_from_slice(&ref_l[SPLIT + 1..]);
    assert_eq!(best_lag(&dropped, &ref_l, SPLIT + 3_000, 1_500, 4), 1);

    let mut inserted: Vec<f32> = ref_l[..SPLIT].to_vec();
    inserted.push(0.0);
    inserted.extend_from_slice(&ref_l[SPLIT..TOTAL - 1]);
    assert_eq!(best_lag(&inserted, &ref_l, SPLIT + 3_000, 1_500, 4), -1);
}
