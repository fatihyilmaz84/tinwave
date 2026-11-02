//! Control behaviour: volume ramp, pause/resume, device switch, failed tracks, queue edits.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use common::*;
use tinwave_playback::{
    EventKind, LeaveCause, PlaybackError, Position, QueueItemId, StartCause, StreamInfo,
};

fn wav_pair(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, TempDir) {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);
    (a, b, dir)
}

fn refs() -> (Vec<f32>, Vec<f32>) {
    let (l, r) = sweep(RATE, TOTAL);
    reference(&l, &r, 16)
}

fn missing(name: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("tinwave-playback-does-not-exist")
        .join(name)
}

#[test]
fn volume_ramps_across_a_block_and_holds_exactly() {
    let (a, b, _dir) = wav_pair("volume");
    let (ref_l, ref_r) = refs();
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let first = h.render_exact(4_800, 480);
    assert_eq!(
        max_err(&first, &ref_l, &ref_r, 0),
        0.0,
        "unity gain is bit-exact"
    );

    h.sink.take();
    h.engine.set_volume(0.25).expect("volume");
    assert_eq!(
        h.sink.take(),
        vec![EventKind::VolumeChanged {
            from: 1.0,
            to: 0.25,
            position: Some(Position {
                item: QueueItemId(1),
                frame: 4_800,
                position_ms: 100
            })
        }]
    );

    // The block after the change ramps linearly 1.0 -> 0.25 (one step per frame, ending exactly on
    // the target); later blocks sit at 0.25.
    let ramp = h.render_exact(480, 480);
    for (i, f) in ramp.chunks_exact(2).enumerate() {
        let gain = 1.0 + (0.25 - 1.0) * (i + 1) as f32 / 480.0;
        assert!((f[0] - gain * ref_l[4_800 + i]).abs() < 1e-6, "frame {i}");
        assert!((f[1] - gain * ref_r[4_800 + i]).abs() < 1e-6, "frame {i}");
    }
    let held = h.render_exact(1_000, 480);
    for (i, f) in held.chunks_exact(2).enumerate() {
        let want = 0.25 * ref_l[5_280 + i];
        assert!((f[0] - want).abs() < 1e-7);
    }
    // Same value again: no event.
    h.sink.take();
    h.engine.set_volume(0.25).expect("volume");
    assert!(h.sink.take().is_empty());
}

#[test]
fn volume_rejects_values_outside_zero_to_one() {
    let mut h = Harness::new();
    for bad in [-0.1f32, 1.5, f32::NAN, f32::INFINITY] {
        assert!(
            matches!(
                h.engine.set_volume(bad),
                Err(PlaybackError::InvalidArgument(_))
            ),
            "{bad}"
        );
    }
    assert_eq!(h.engine.volume(), 1.0);
}

#[test]
fn pause_fades_out_then_holds_the_position_and_resume_continues_at_the_next_frame() {
    let (a, b, _dir) = wav_pair("pause");
    let (ref_l, ref_r) = refs();
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(4_800, 480);

    h.sink.take();
    h.engine.pause();
    h.engine.pause(); // second call is a no-op
    assert!(h.engine.is_paused());
    let kinds = h.sink.take();
    assert_eq!(kinds.len(), 1);
    assert!(matches!(&kinds[0], EventKind::Paused { position: Some(p) } if p.frame == 4_800));

    // Fade-out block: audio from the same frames, scaled down to silence.
    h.wait_buffered(480);
    let (fade, real) = h.render_block(480);
    assert_eq!(real, 480);
    let mut g = 1.0f32;
    for (i, f) in fade.chunks_exact(2).enumerate() {
        let want = ref_l[4_800 + i];
        if want.abs() > 0.02 {
            let gain = f[0] / want;
            assert!(gain <= g + 1e-3);
            g = gain;
        }
    }
    assert!(g < 0.05, "fade ends at silence, got {g}");
    // Then nothing is consumed: all-zero blocks, position frozen.
    let frozen = h.engine.position().expect("position");
    for _ in 0..20 {
        let (block, real) = h.render_block(480);
        assert_eq!(real, 0);
        assert!(block.iter().all(|&s| s == 0.0));
    }
    assert_eq!(h.engine.position().expect("position"), frozen);
    assert_eq!(frozen.frame, 5_280);

    h.sink.take();
    h.engine.resume();
    let kinds = h.sink.take();
    assert!(matches!(&kinds[0], EventKind::Resumed { position: Some(p) } if p.frame == 5_280));
    // Fade-in, then exactly the following frames.
    h.wait_buffered(480);
    let (fade_in, _) = h.render_block(480);
    assert!(fade_in[..4].iter().all(|s| s.abs() < 0.02));
    let rest = h.render_exact(2_000, 480);
    assert_eq!(max_err(&rest, &ref_l, &ref_r, 5_760), 0.0);
}

#[test]
fn switching_device_resumes_at_the_exact_frame() {
    let (a, b, _dir) = wav_pair("switch");
    let (ref_l, ref_r) = refs();
    let mut h = Harness::new();
    h.latency = 960;
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let before = h.render_exact(14_400, 480);
    assert_eq!(max_err(&before, &ref_l, &ref_r, 0), 0.0);

    h.sink.take();
    h.engine
        .switch_device(Some("speakers-2".into()))
        .expect("switch");
    assert_eq!(
        h.out.opens(),
        vec![(None, RATE), (Some("speakers-2".into()), RATE)]
    );
    assert_eq!(
        h.sink.take(),
        vec![EventKind::OutputReopened {
            device: Some("speakers-2".into()),
            info: StreamInfo::unknown(),
        }]
    );
    h.settle();

    // Frames still in the old device buffer (the 960 latency frames) are played again by the new
    // stream, so the music continues from exactly the frame that was being heard.
    let after = h.render_exact(5_000, 480);
    assert_eq!(max_err(&after, &ref_l, &ref_r, 14_400 - 960), 0.0);
    let position = h.engine.position().expect("position");
    assert_eq!(position.item, QueueItemId(1));
    assert_eq!(position.frame, 14_400 - 960 + 5_000 - 960);

    // No track-change noise and no spurious underrun from the re-open: the new stream starts
    // unprimed, so callbacks before the decoder has switched over are not starvation.
    let events = h.sink.take();
    assert!(
        events
            .iter()
            .all(|k| matches!(k, EventKind::PositionMark { .. })),
        "{events:?}"
    );
}

#[test]
fn failed_switch_reports_and_a_retry_recovers() {
    let (a, b, _dir) = wav_pair("switch-fail");
    let (ref_l, ref_r) = refs();
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(4_800, 480);
    h.sink.take();

    h.out.fail_next_open();
    let err = h
        .engine
        .switch_device(Some("gone".into()))
        .expect_err("open refused");
    assert!(matches!(err, PlaybackError::Output(_)));
    assert!(!h.out.is_open());
    assert_eq!(h.sink.take(), vec![EventKind::OutputLost]);

    h.engine.switch_device(None).expect("retry");
    h.settle();
    let rec = h.render_exact(1_000, 500);
    assert_eq!(max_err(&rec, &ref_l, &ref_r, 4_800), 0.0);
}

#[test]
fn lost_output_is_reported_once() {
    let mut h = Harness::new();
    h.engine.poll();
    assert!(h.sink.take().is_empty());
    h.out.set_lost(true);
    h.engine.poll();
    h.engine.poll();
    assert_eq!(h.sink.take(), vec![EventKind::OutputLost]);
}

#[test]
fn unplayable_tracks_are_reported_and_skipped() {
    let dir = TempDir::new("failures");
    let (l, r) = sweep(RATE, TOTAL);
    let good = dir.path("good.wav");
    write_wav(&good, RATE, 16, &l[..4_800], &r[..4_800]);
    let wrong_rate = dir.path("rate.wav");
    write_wav(&wrong_rate, 44_100, 16, &l[..4_800], &r[..4_800]);
    let garbage = dir.path("garbage.wav");
    std::fs::write(&garbage, vec![0x55u8; 4_096]).expect("write");

    let mut h = Harness::new();
    h.engine
        .load(
            vec![
                track(1, missing("missing.wav")),
                track(2, &wrong_rate),
                track(3, &garbage),
                track(4, &good),
            ],
            0,
            0,
        )
        .expect("load");
    let rec = h.render_exact(4_800, 480);
    h.finish();
    let (ref_l, ref_r) = reference(&l, &r, 16);
    assert_eq!(max_err(&rec, &ref_l, &ref_r, 0), 0.0);

    let kinds = h.sink.kinds();
    let failed: Vec<(u64, &PlaybackError)> = kinds
        .iter()
        .filter_map(|k| match k {
            EventKind::TrackFailed { item, error } => Some((item.0, error)),
            _ => None,
        })
        .collect();
    assert_eq!(failed.len(), 3, "{kinds:?}");
    assert!(matches!(failed[0], (1, PlaybackError::Open(_))));
    assert!(matches!(
        failed[1],
        (
            2,
            PlaybackError::SampleRateMismatch {
                track_rate: 44_100,
                output_rate: 48_000
            }
        )
    ));
    assert!(matches!(failed[2], (3, PlaybackError::Probe(_))));
    // The command announced item 1; item 4 took over, which is reported as an advance.
    assert!(kinds.contains(&EventKind::TrackStarted {
        item: QueueItemId(4),
        frame: 0,
        cause: StartCause::Advance
    }));
}

#[test]
fn a_queue_where_nothing_plays_ends_immediately() {
    let mut h = Harness::new();
    h.engine
        .load(
            vec![
                track(1, missing("nothing-1.wav")),
                track(2, missing("nothing-2.wav")),
            ],
            0,
            0,
        )
        .expect("load");
    h.settle();
    h.engine.poll();
    let kinds = h.sink.kinds();
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, EventKind::TrackFailed { .. }))
            .count(),
        2
    );
    assert!(kinds.contains(&EventKind::QueueEnded));
}

#[test]
fn append_continues_gaplessly_and_restarts_an_ended_queue() {
    let (a, b, _dir) = wav_pair("append");
    let (ref_l, ref_r) = refs();
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    h.engine.append(vec![track(2, &b)]).expect("append");
    let rec = h.render_exact(TOTAL, 480);
    assert_eq!(max_err(&rec, &ref_l, &ref_r, 0), 0.0);
    h.finish();

    // After the end: appended items start playing again.
    h.sink.take();
    h.engine
        .append(vec![track(3, &b)])
        .expect("append after the end");
    let rec = h.render_exact(1_000, 500);
    assert_eq!(max_err(&rec, &ref_l, &ref_r, SPLIT), 0.0);
    let kinds = h.sink.take();
    assert!(kinds.contains(&EventKind::TrackStarted {
        item: QueueItemId(3),
        frame: 0,
        cause: StartCause::Advance
    }));
}

#[test]
fn stop_clears_the_queue_without_ending_it() {
    let (a, b, _dir) = wav_pair("stop");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(2_400, 480);
    h.sink.take();
    h.engine.stop().expect("stop");
    assert_eq!(
        h.sink.take(),
        vec![EventKind::TrackLeft {
            item: QueueItemId(1),
            frame: 2_400,
            cause: LeaveCause::Stopped
        }]
    );
    h.settle();
    for _ in 0..10 {
        let (_, real) = h.render_block(480);
        assert_eq!(real, 0);
    }
    assert!(h.engine.position().is_none());
    assert!(h
        .sink
        .take()
        .iter()
        .all(|k| !matches!(k, EventKind::QueueEnded)));
    assert!(matches!(
        h.engine.seek(10),
        Err(PlaybackError::NothingLoaded)
    ));
}

#[test]
fn invalid_queues_are_rejected() {
    let mut h = Harness::new();
    assert!(matches!(
        h.engine.load(vec![], 0, 0),
        Err(PlaybackError::InvalidArgument(_))
    ));
    assert!(matches!(
        h.engine.load(vec![track(1, "a.wav")], 1, 0),
        Err(PlaybackError::InvalidArgument(_))
    ));
    assert!(matches!(
        h.engine
            .load(vec![track(1, "a.wav"), track(1, "b.wav")], 0, 0),
        Err(PlaybackError::InvalidArgument(_))
    ));
}

#[test]
fn shutdown_joins_the_decoder_thread() {
    let (a, b, _dir) = wav_pair("shutdown");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, a), track(2, b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(480, 480);
    h.engine.shutdown();
}
