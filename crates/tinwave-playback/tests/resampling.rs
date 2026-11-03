//! Resampling in the decoder thread (T-0019, ADR 0005 decision 5): tracks whose rate differs from
//! the stream rate play at the right pitch and length, gaplessly, across rate changes, seeks,
//! queue edits and device changes.
//!
//! The reference is analytic: the sweep of `common::sweep` is a continuous function, so the 48 kHz
//! stream must equal it sampled at 48 kHz, whatever rate the files were stored at.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use common::*;
use tinwave_playback::{EventKind, LeaveCause, QueueItemId};

/// The sweep is -20 dBFS (0.1). A resampler that is exact to 1e-3 of that is exact to 1e-4 of
/// full scale, far below anything audible; a missing boundary (reset filter) is 10 times worse.
const TOLERANCE: f32 = 1.0e-3;
/// Frames next to the start of a stream the filter has not settled on (it starts against silence).
const EDGE: usize = 600;
/// Frames either side of a rate change where a resampler starts or stops against silence. The
/// test sweep is a continuous tone cut in two, so there the filter meets a hard edge; real
/// tracks end and start near silence. Measured: the transient is over within 80 frames.
const JOIN: usize = 100;
/// The most the hard edge may disturb the sweep (amplitude 0.1) at a rate change: it is a few
/// frames of filter ringing, not a gap, a repeat or a dropout.
const JOIN_TOLERANCE: f32 = 0.05;

/// `rec` against the ideal sweep, `rec[0]` being stream frame `start`; the first and last `EDGE`
/// frames are not judged.
fn error_vs_ideal(rec: &[f32], start: usize) -> f32 {
    let (ideal_l, ideal_r) = ideal_sweep();
    let frames = rec.len() / 2;
    max_err(
        &rec[2 * EDGE..2 * (frames - EDGE)],
        &ideal_l,
        &ideal_r,
        start + EDGE,
    )
}

/// Same, for a stream that starts mid-way and whose first frames are the filter settling.
fn error_after_start(rec: &[f32], start: usize) -> f32 {
    let (ideal_l, ideal_r) = ideal_sweep();
    max_err(&rec[2 * EDGE..], &ideal_l, &ideal_r, start + EDGE)
}

/// Largest left-channel error of a whole-sweep recording within `JOIN` frames of `centre`, and
/// outside `JOIN` frames of every one of `centres` (past the first and last `EDGE` frames).
fn error_near_and_away(rec: &[f32], centres: &[usize]) -> (f32, f32) {
    let (ideal_l, _) = ideal_sweep();
    let (mut near, mut away) = (0.0f32, 0.0f32);
    for i in EDGE..rec.len() / 2 - EDGE {
        let e = (rec[2 * i] - ideal_l[i]).abs();
        if centres.iter().any(|&c| i.abs_diff(c) <= JOIN) {
            near = near.max(e);
        } else {
            away = away.max(e);
        }
    }
    (near, away)
}

fn lag_at(rec: &[f32], centre: usize) -> i64 {
    let (ideal_l, _) = ideal_sweep();
    let (rec_l, _) = deinterleave(rec);
    best_lag(&rec_l, &ideal_l, centre, 200, 4)
}

#[test]
fn a_44k_pair_on_a_48k_stream_is_gapless_within_one_frame() {
    let dir = TempDir::new("resample-pair");
    // 2 s at 44.1 kHz cut 777 frames after one second: not aligned to anything.
    let split = 44_100 + 777;
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_sweep_piece(&a, 44_100, 0, split, 2);
    write_sweep_piece(&b, 44_100, split, 88_200, 2);

    let mut h = Harness::new();
    h.latency = 480;
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    // Exactly the converted length comes out, with no underrun silence in between.
    let rec = h.render_exact(TOTAL, 512);
    h.finish();

    let boundary = (split as f64 * 48_000.0 / 44_100.0).round() as usize;
    assert_eq!(
        lag_at(&rec, boundary),
        0,
        "misaligned at the boundary: a gap or an overlap"
    );
    assert_eq!(lag_at(&rec, boundary - 3_000), 0, "head");
    assert_eq!(lag_at(&rec, boundary + 3_000), 0, "tail");
    let err = error_vs_ideal(&rec, 0);
    assert!(err < TOLERANCE, "waveform deviates by {err}");

    // The boundary the engine reports is the scaled input frame, to the frame.
    let kinds = h.sink.kinds();
    assert!(
        kinds.contains(&EventKind::TrackLeft {
            item: QueueItemId(1),
            frame: boundary as u64,
            cause: LeaveCause::Finished,
        }),
        "{kinds:?}"
    );
    assert!(kinds.contains(&EventKind::TrackLeft {
        item: QueueItemId(2),
        frame: (TOTAL - boundary) as u64,
        cause: LeaveCause::Finished,
    }));
    assert_paired(&kinds, false);
}

#[test]
fn the_boundary_between_same_rate_tracks_has_no_filter_restart() {
    // Same pair, error measured in a small window around the join: this is where an independent
    // resampler per track (a reset at the boundary) shows up as a burst of error.
    let dir = TempDir::new("resample-boundary");
    let split = 40_001;
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_sweep_piece(&a, 44_100, 0, split, 2);
    write_sweep_piece(&b, 44_100, split, 88_200, 2);
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    let rec = h.render_exact(TOTAL, 512);

    let boundary = (split as f64 * 48_000.0 / 44_100.0).round() as usize;
    let (ideal_l, ideal_r) = ideal_sweep();
    let window = 2 * (boundary - 200)..2 * (boundary + 200);
    let err = max_err(&rec[window], &ideal_l, &ideal_r, boundary - 200);
    assert!(err < TOLERANCE, "error around the boundary {err}");
}

#[test]
fn rate_changes_between_tracks_play_without_a_glitch() {
    let dir = TempDir::new("resample-rates");
    let [a, b, c] = rate_change_files(&dir);
    let mut h = Harness::new();
    h.latency = 480;
    h.engine
        .load(vec![track(1, &a), track(2, &b), track(3, &c)], 0, 0)
        .expect("load");
    let rec = h.render_exact(TOTAL, 441);
    h.finish();

    // 48 -> 44.1 at 33,600 and 44.1 -> 96 at 62,400, to the frame.
    let joins = [33_600, 62_400];
    for centre in joins {
        assert_eq!(lag_at(&rec, centre), 0, "boundary at {centre}");
    }
    let (near, away) = error_near_and_away(&rec, &joins);
    assert!(away < TOLERANCE, "waveform deviates by {away}");
    assert!(near < JOIN_TOLERANCE, "rate change glitch of {near}");

    let kinds = h.sink.kinds();
    for (item, frame) in [(1, 33_600), (2, 28_800), (3, 33_600)] {
        assert!(
            kinds.contains(&EventKind::TrackLeft {
                item: QueueItemId(item),
                frame,
                cause: LeaveCause::Finished,
            }),
            "item {item}: {kinds:?}"
        );
    }
    assert!(!kinds
        .iter()
        .any(|k| matches!(k, EventKind::Underrun { .. })));
    assert_paired(&kinds, false);
}

#[test]
fn a_mono_file_is_resampled_to_both_channels() {
    let dir = TempDir::new("resample-mono");
    let mono = dir.path("mono.wav");
    write_sweep_piece(&mono, 44_100, 0, 88_200, 1);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &mono)], 0, 0).expect("load");
    let rec = h.render_exact(TOTAL, 480);
    let (ideal_l, _) = ideal_sweep();
    let frames = rec.len() / 2;
    for i in EDGE..frames - EDGE {
        assert_eq!(rec[2 * i], rec[2 * i + 1], "channels differ at {i}");
        assert!((rec[2 * i] - ideal_l[i]).abs() < TOLERANCE, "frame {i}");
    }
}

#[test]
fn a_start_frame_and_a_seek_are_stream_frames_in_a_resampled_track() {
    let dir = TempDir::new("resample-seek");
    let [a, b, c] = rate_change_files(&dir);
    let queue = || vec![track(1, &a), track(2, &b), track(3, &c)];

    // Start inside the 44.1 kHz track, 9,920 stream frames in (a multiple of 160, so the file
    // frame, 9,114, is a whole number and the audio is aligned to the frame; any other target
    // lands within half a file frame of it).
    let mut h = Harness::new();
    h.engine.load(queue(), 1, 9_920).expect("load");
    let rec = h.render_exact(12_000, 480);
    let err = error_after_start(&rec, 33_600 + 9_920);
    assert!(err < TOLERANCE, "after a start frame: {err}");
    let position = h.engine.position().expect("playing");
    assert_eq!(
        (position.item, position.frame),
        (QueueItemId(2), 21_920),
        "position counts stream frames of the track"
    );
    assert_eq!(position.position_ms, 21_920 * 1000 / 48_000);

    // Seek inside the 96 kHz track to a stream frame.
    h.engine.load(queue(), 2, 0).expect("load");
    let _ = h.render_exact(1_000, 480);
    h.engine.seek(20_000).expect("seek");
    let rec = h.render_exact(6_000, 480);
    let err = error_after_start(&rec, 62_400 + 20_000);
    assert!(err < TOLERANCE, "after a seek: {err}");
}

#[test]
fn a_seek_to_the_last_frame_of_a_downsampled_track_is_not_out_of_range() {
    // 96 kHz, 96,001 frames: 48,000.5 stream frames, which is 48,001 once rounded, and twice that
    // is one frame past the file. Seeking to the end is legal and must not fail.
    let dir = TempDir::new("resample-end");
    let (odd, next) = (dir.path("odd.wav"), dir.path("next.wav"));
    write_sweep_piece(&odd, 96_000, 0, 96_001, 2);
    write_sweep_piece(&next, 48_000, 0, 4_800, 2);
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &odd), track(2, &next)], 0, 48_001)
        .expect("load");
    h.settle();
    h.run_until("the next track started", |h| {
        h.saw(|k| matches!(k, EventKind::TrackStarted { item, .. } if *item == QueueItemId(2)))
    });
    assert!(
        !h.saw(|k| matches!(k, EventKind::SeekFailed { .. })),
        "{:?}",
        h.sink.kinds()
    );
}

#[test]
fn appending_to_a_finished_queue_resamples_the_new_track_from_its_start() {
    let dir = TempDir::new("resample-append");
    let [a, b, _] = rate_change_files(&dir);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    let rec = h.render_exact(33_600, 480);
    h.finish();
    assert!(error_vs_ideal(&rec, 0) < TOLERANCE);

    // The queue has ended; the 44.1 kHz track continues the stream with a fresh resampler.
    h.engine.append(vec![track(2, &b)]).expect("append");
    let rec = h.render_exact(28_800, 480);
    let err = error_vs_ideal(&rec, 33_600);
    assert!(err < TOLERANCE, "appended track deviates by {err}");
}

#[test]
fn a_queue_edit_keeps_the_resampler_running_across_the_new_tail() {
    // Three pieces of the sweep at 44.1 kHz. The tail starts as [third] and is replaced by
    // [second, third] while the first plays: the edit must not restart the resampler, so the
    // joins are as clean as in a queue that was never edited.
    let dir = TempDir::new("resample-edit");
    let paths = [dir.path("p1.wav"), dir.path("p2.wav"), dir.path("p3.wav")];
    write_sweep_piece(&paths[0], 44_100, 0, 20_001, 2);
    write_sweep_piece(&paths[1], 44_100, 20_001, 60_000, 2);
    write_sweep_piece(&paths[2], 44_100, 60_000, 88_200, 2);
    let mut h = Harness::new();
    h.latency = 480;
    h.engine
        .load(vec![track(1, &paths[0]), track(3, &paths[2])], 0, 0)
        .expect("load");
    let mut rec = h.render_exact(4_800, 480);
    h.engine
        .replace_after(
            QueueItemId(1),
            vec![track(2, &paths[1]), track(4, &paths[2])],
        )
        .expect("edit");
    rec.extend(h.render_exact(TOTAL - 4_800, 480));
    h.finish();
    let err = error_vs_ideal(&rec, 0);
    assert!(err < TOLERANCE, "waveform deviates by {err}");
    assert!(h.saw(|k| matches!(k, EventKind::QueueEdited { .. })));
    assert_paired(&h.sink.kinds(), false);
}

#[test]
fn a_device_change_resumes_a_resampled_track_at_the_same_frame() {
    let dir = TempDir::new("resample-device");
    let [_, b, c] = rate_change_files(&dir);
    let mut h = Harness::new();
    h.engine
        .load(vec![track(2, &b), track(3, &c)], 0, 0)
        .expect("load");
    // 9,600 frames in, a multiple of 160: the file frame is a whole number (see above).
    let _ = h.render_exact(9_600, 480);
    h.engine
        .switch_device(Some("other".into()))
        .expect("switch");
    let resumed = h.engine.position().expect("position").frame as usize;
    assert_eq!(
        resumed, 9_600,
        "the device change resumes where it was heard"
    );
    let rec = h.render_exact(6_000, 480);
    let err = error_after_start(&rec, 33_600 + resumed);
    assert!(err < TOLERANCE, "after the device change: {err}");
}
