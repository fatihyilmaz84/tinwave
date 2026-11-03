//! Exclusive mode follows the sample rate of the files (T-0057, ADR 0005 decision 5): at a track
//! of another rate the output is reopened at that rate, nothing is resampled, and the samples
//! reach the callback bit for bit. A device that refuses a rate gets the engine's own rate and
//! the decoder's resampler instead, and the refusal is reported once.
//!
//! The device is `ManualOutput` / the fake pump device: no real audio device is opened. The test
//! thread plays the audio callback and polls the engine, as the controller thread does.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use tinwave_core::{ManualClock, UtcMillis};
use tinwave_playback::{
    Engine, EngineConfig, EventKind, FsOpener, LeaveCause, PumpStep, QueueItemId, SampleFormat,
    StreamInfo, StreamMode,
};

const R44: u32 = 44_100;
const R96: u32 = 96_000;
/// Every file is one second long, so at the 48 kHz timeline it is 48,000 frames.
const SECOND: usize = 48_000;
const BLOCK: usize = 480;

/// A one second 24-bit sweep at `rate`, written to `path`; returns the frames a decoder must
/// produce for it (interleaved), which are the file's samples exactly.
fn sweep_file(path: &Path, rate: u32) -> Vec<f32> {
    let (l, r) = sweep(rate, rate as usize);
    write_wav(path, rate, 24, &l, &r);
    interleave(&reference(&l, &r, 24))
}

fn interleave((l, r): &(Vec<f32>, Vec<f32>)) -> Vec<f32> {
    l.iter().zip(r).flat_map(|(&l, &r)| [l, r]).collect()
}

/// What `play_to_end` saw.
struct Played {
    /// The frames of real audio, in order, with the silence between them left out.
    audio: Vec<f32>,
    /// Real frames the engine had played when each `OutputReopened` appeared, and the frames of
    /// silence the callback had produced since its last frame of audio.
    at_reopen: Vec<(u64, u64)>,
}

/// Play the audio callback (480-frame blocks) and poll until the queue has ended. `at_least` is
/// a number of frames the queue is known to hold: until that many have been played the callback
/// waits for the decoder to fill a block (a callback that came first would be an underrun),
/// except while the engine is reopening the stream, when the device keeps calling with silence.
fn play_to_end(h: &mut Harness, at_least: usize) -> Played {
    let mut played = Played {
        audio: Vec::new(),
        at_reopen: Vec::new(),
    };
    let mut block = vec![0.0f32; BLOCK * 2];
    let mut silence = 0u64;
    for _ in 0..60_000 {
        for _ in 0..30_000 {
            let ready = played.audio.len() / 2 >= at_least
                || h.out.buffered_frames().unwrap_or(0) >= BLOCK
                || h.engine.reopen_pending();
            if ready {
                break;
            }
            h.engine.poll();
            std::thread::sleep(Duration::from_micros(100));
        }
        let real = h.out.render(&mut block, h.latency).unwrap_or(0);
        played.audio.extend_from_slice(&block[..real * 2]);
        silence = match real {
            0 => silence + BLOCK as u64,
            r if r < BLOCK => (BLOCK - r) as u64,
            _ => 0,
        };
        h.engine.poll();
        let reopened = h
            .sink
            .kinds()
            .iter()
            .filter(|k| matches!(k, EventKind::OutputReopened { .. }))
            .count();
        while played.at_reopen.len() < reopened {
            played
                .at_reopen
                .push((h.engine.rate_stats().wall_frames, silence));
        }
        if h.saw(|k| *k == EventKind::QueueEnded) {
            return played;
        }
        if real < BLOCK {
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    panic!("the queue never ended; events: {:?}", h.sink.kinds());
}

/// The `OutputReopened` infos, in order.
fn reopened(h: &Harness) -> Vec<StreamInfo> {
    h.sink
        .kinds()
        .into_iter()
        .filter_map(|k| match k {
            EventKind::OutputReopened { info, .. } => Some(info),
            _ => None,
        })
        .collect()
}

fn rates(h: &Harness) -> Vec<u32> {
    h.out.opens().into_iter().map(|(_, rate)| rate).collect()
}

fn left_after(items: &[EventKind], item: u64) -> Option<(u64, LeaveCause)> {
    items.iter().find_map(|k| match k {
        EventKind::TrackLeft {
            item: i,
            frame,
            cause,
        } if *i == QueueItemId(item) => Some((*frame, *cause)),
        _ => None,
    })
}

/// Underruns reported before the last track started. (The very end of a queue is a race the
/// tests do not judge: a callback can take the last frames a moment before the decoder marks the
/// end of the stream, which counts as starvation. Everything before it, the stream replacements
/// included, must be free of it.)
fn underruns(kinds: &[EventKind]) -> usize {
    let last = kinds
        .iter()
        .rposition(|k| matches!(k, EventKind::TrackStarted { .. }))
        .unwrap_or(kinds.len());
    kinds[..last]
        .iter()
        .filter(|k| matches!(k, EventKind::Underrun { .. }))
        .count()
}

#[test]
fn a_rate_change_reopens_the_output_at_the_file_rate_and_both_tracks_are_bit_exact() {
    let dir = TempDir::new("excl-bitexact");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    let (ref_a, ref_b) = (sweep_file(&a, R44), sweep_file(&b, R96));

    let mut h = Harness::exclusive(None);
    h.latency = 480;
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle_polling();
    let played = play_to_end(&mut h, 140_100 - BLOCK);

    // The stream followed the files: the engine's own rate, then 44.1 kHz, then 96 kHz.
    assert_eq!(rates(&h), [48_000, R44, R96]);
    // Nothing was resampled, stretched, scaled or dithered: the samples are the files'.
    let mut expected = ref_a.clone();
    expected.extend_from_slice(&ref_b);
    assert_eq!(played.audio.len(), expected.len());
    assert!(played.audio == expected, "samples differ from the files");

    // The books agree: one wall frame per source frame, no rate flush.
    let stats = h.engine.rate_stats();
    assert_eq!(stats.wall_frames, (R44 + R96) as u64);
    assert_eq!(stats.source_micro_frames, (R44 + R96) as u64 * 1_000_000);
    assert_eq!((stats.effective_rate, stats.flushes), (1.0, 0));

    // Two reopens, exclusive both times, neither a refusal.
    let infos = reopened(&h);
    assert_eq!(infos.len(), 2);
    for info in &infos {
        assert_eq!(info.mode, StreamMode::Exclusive);
        assert_eq!(info.fallback, None);
    }
    // The track events are the ones of any gapless hand-off, in timeline frames (48 kHz).
    let kinds = h.sink.kinds();
    assert_eq!(
        left_after(&kinds, 1),
        Some((SECOND as u64, LeaveCause::Finished))
    );
    assert_eq!(
        left_after(&kinds, 2),
        Some((SECOND as u64, LeaveCause::Finished))
    );
    assert_eq!(underruns(&kinds), 0, "underruns");
    assert_paired(&kinds, false);
    let order: Vec<&str> = kinds
        .iter()
        .filter_map(|k| match k {
            EventKind::TrackStarted { item, .. } => {
                Some(if item.0 == 1 { "start 1" } else { "start 2" })
            }
            EventKind::TrackLeft { item, .. } => {
                Some(if item.0 == 1 { "left 1" } else { "left 2" })
            }
            EventKind::OutputReopened { .. } => Some("reopened"),
            EventKind::QueueEnded => Some("ended"),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        ["start 1", "reopened", "reopened", "left 1", "start 2", "left 2", "ended"]
    );

    // The old stream is not closed under the end of the first track: it has played its last
    // frame and its device buffer (480 frames of latency here) has emptied.
    let (wall, silence) = played.at_reopen[1];
    assert_eq!(
        wall, R44 as u64,
        "the first track had been played to its last frame"
    );
    assert!(
        silence >= 480,
        "reopened with {silence} silent frames played; the tail was cut"
    );
}

#[test]
fn same_rate_tracks_in_exclusive_mode_never_reopen_and_stay_gapless() {
    // The sweep of the gapless tests, cut in three pieces at a rate that is not the engine's.
    let dir = TempDir::new("excl-same-rate");
    let (l, r) = sweep(R44, 2 * R44 as usize);
    let cuts = [0, 30_001, 61_777, 2 * R44 as usize];
    let files: Vec<_> = (0..3).map(|i| dir.path(&format!("p{i}.wav"))).collect();
    for (i, file) in files.iter().enumerate() {
        write_wav(
            file,
            R44,
            24,
            &l[cuts[i]..cuts[i + 1]],
            &r[cuts[i]..cuts[i + 1]],
        );
    }
    let expected = interleave(&reference(&l, &r, 24));

    let mut h = Harness::exclusive(None);
    h.engine
        .load(
            files.iter().zip(1..).map(|(f, id)| track(id, f)).collect(),
            0,
            0,
        )
        .expect("load");
    h.settle_polling();
    // `render_exact` fails on any starved callback: the hand-offs have no gap.
    let rec = h.render_exact(2 * R44 as usize, 441);
    h.finish();
    assert!(rec == expected, "samples differ from the files");
    assert_eq!(
        rates(&h),
        [48_000, R44],
        "one reopen, for the first track only"
    );
    assert_eq!(reopened(&h).len(), 1);
    assert_eq!(underruns(&h.sink.kinds()), 0, "underruns");
}

#[test]
fn shared_mode_keeps_one_stream_at_the_engine_rate_and_resamples() {
    let dir = TempDir::new("excl-shared");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    sweep_file(&a, R44);
    sweep_file(&b, R96);
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle_polling();
    let played = play_to_end(&mut h, 2 * SECOND - BLOCK);
    assert_eq!(rates(&h), [48_000]);
    assert!(reopened(&h).is_empty());
    assert_eq!(
        played.audio.len() / 2,
        2 * SECOND,
        "both tracks at the stream rate"
    );
}

#[test]
fn a_load_and_a_seek_into_a_track_of_another_rate_reopen_and_start_at_the_right_frame() {
    let dir = TempDir::new("excl-load");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    let (_, ref_b) = (sweep_file(&a, R44), sweep_file(&b, R96));
    let queue = || vec![track(1, &a), track(2, &b)];

    let mut h = Harness::exclusive(None);
    h.engine.load(queue(), 0, 0).expect("load");
    h.settle_polling();
    let _ = h.render_exact(1_000, 480);
    assert_eq!(rates(&h), [48_000, R44]);

    // Start 24,000 timeline frames in: frame 48,000 of the 96 kHz file.
    h.engine.load(queue(), 1, 24_000).expect("load");
    h.settle_polling();
    assert_eq!(rates(&h), [48_000, R44, R96]);
    let rec = h.render_exact(9_600, 480);
    assert!(
        rec == ref_b[2 * 48_000..2 * (48_000 + 9_600)],
        "load into the 96 kHz file"
    );
    let position = h.engine.position().expect("playing");
    assert_eq!(
        (position.item, position.frame),
        (QueueItemId(2), 24_000 + 4_800)
    );
    assert_eq!(position.position_ms, (24_000 + 4_800) * 1000 / 48_000);

    // A seek inside the file needs no new stream.
    h.engine.seek(36_000).expect("seek");
    h.settle_polling();
    let rec = h.render_exact(4_800, 480);
    assert!(
        rec == ref_b[2 * 72_000..2 * (72_000 + 4_800)],
        "seek in the 96 kHz file"
    );
    assert_eq!(
        rates(&h),
        [48_000, R44, R96],
        "no reopen for a seek in the same file"
    );

    // A seek back to the first file (another rate) does.
    h.engine.load(queue(), 0, 0).expect("load");
    h.settle_polling();
    assert_eq!(rates(&h), [48_000, R44, R96, R44]);
}

#[test]
fn positions_and_marks_continue_across_the_reopen() {
    let dir = TempDir::new("excl-position");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    sweep_file(&a, R44);
    sweep_file(&b, R96);
    let mut h = Harness::exclusive(None);
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle_polling();

    // Play the whole first file and 9,600 frames of the second (4,800 timeline frames).
    let target = R44 as usize + 9_600;
    let mut got = 0;
    let mut block = vec![0.0f32; BLOCK * 2];
    for _ in 0..60_000 {
        let want = BLOCK.min(target - got);
        let real = h
            .out
            .render(&mut block[..want * 2], 0)
            .expect("stream open");
        got += real;
        h.engine.poll();
        if got == target {
            break;
        }
        if real < want {
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    assert_eq!(got, target);
    let position = h.engine.position().expect("playing");
    assert_eq!((position.item, position.frame), (QueueItemId(2), 4_800));
    // The 10 Hz marks follow heard audio at either rate: about one per tenth of a second of the
    // first file, in order, and the first one of the second file is 0.1 s (4,800 frames) in.
    let marks: Vec<_> = h
        .sink
        .kinds()
        .into_iter()
        .filter_map(|k| match k {
            EventKind::PositionMark { position } => Some(position),
            _ => None,
        })
        .collect();
    assert!(
        marks
            .windows(2)
            .all(|w| (w[0].item.0, w[0].frame) < (w[1].item.0, w[1].frame)),
        "marks go forward: {marks:?}"
    );
    assert!(marks.iter().filter(|m| m.item == QueueItemId(1)).count() >= 9);
    let second: Vec<_> = marks.iter().filter(|m| m.item == QueueItemId(2)).collect();
    assert_eq!(second.len(), 1, "{marks:?}");
    assert_eq!(second[0].frame, 4_800);
}

#[test]
fn the_position_stays_on_the_old_track_until_the_new_stream_is_heard() {
    // The device reports 480 frames of latency: after the reopen the first frames of the second
    // file are consumed 10 ms before they are heard, and until then the listener is still at the
    // end of the first file, not nowhere.
    let dir = TempDir::new("excl-position-gap");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    sweep_file(&a, R44);
    sweep_file(&b, R96);
    let mut h = Harness::exclusive(None);
    h.latency = 480;
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle_polling();

    let mut block = vec![0.0f32; 100 * 2];
    let mut wall = 0;
    let mut seen_between = false;
    let mut seen_second = false;
    for _ in 0..60_000 {
        let want_ready = h.out.buffered_frames().unwrap_or(0) >= 100 || h.engine.reopen_pending();
        if want_ready {
            h.out.render(&mut block, h.latency).expect("stream open");
            h.engine.poll();
            wall = h.engine.rate_stats().wall_frames;
            let position = h.engine.position().expect("a position at every moment");
            if wall > R44 as u64 + 100 && wall < R44 as u64 + 400 {
                // Consumed, not heard yet: the old track, near its end, never past it.
                assert_eq!(position.item, QueueItemId(1), "at {wall} frames");
                assert!(position.frame <= SECOND as u64);
                seen_between = true;
            }
            if wall >= R44 as u64 + 1_000 {
                assert_eq!(position.item, QueueItemId(2));
                seen_second = true;
                break;
            }
        } else {
            h.engine.poll();
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    assert!(seen_between && seen_second, "wall frames {wall}");
}

#[test]
fn a_load_while_the_decoder_waits_for_a_stream_cancels_the_wait() {
    let dir = TempDir::new("excl-cancel-park");
    let (a, b, c) = (dir.path("a.wav"), dir.path("b.wav"), dir.path("c.wav"));
    // A is half a second, so the decoder has pushed all of it and is parked for B at once.
    let (l, r) = sweep(R44, R44 as usize);
    write_wav(&a, R44, 24, &l[..22_050], &r[..22_050]);
    sweep_file(&b, R96);
    let ref_c = sweep_file(&c, R44);

    let mut h = Harness::exclusive(None);
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle_polling();
    // Wait for the decoder to ask for 96 kHz (nothing has been played: the old stream is not
    // empty yet, so the engine does not answer).
    for _ in 0..30_000 {
        if h.engine.reopen_pending() {
            break;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    assert!(
        h.engine.reopen_pending(),
        "the decoder never parked for the second track"
    );

    // The listener picks another track, at the rate of the stream that is open.
    h.engine.load(vec![track(3, &c)], 0, 0).expect("load");
    h.settle_polling();
    let rec = h.render_exact(4_800, 480);
    assert!(rec == ref_c[..2 * 4_800]);
    assert!(
        !h.engine.reopen_pending(),
        "the request of the replaced queue is gone"
    );
    assert_eq!(
        rates(&h),
        [48_000, R44],
        "the stream was never opened at 96 kHz"
    );
}

#[test]
fn a_refused_rate_plays_resampled_at_the_engine_rate_and_is_reported_once() {
    let dir = TempDir::new("excl-refused");
    let paths: Vec<_> = ["a", "b", "c", "d"]
        .iter()
        .map(|n| dir.path(&format!("{n}.wav")))
        .collect();
    let rates_of = [R44, R96, R44, R96];
    let refs: Vec<_> = paths
        .iter()
        .zip(rates_of)
        .map(|(p, r)| sweep_file(p, r))
        .collect();

    // The device takes the engine's rate and 44.1 kHz in exclusive mode, not 96 kHz.
    let mut h = Harness::exclusive(Some(&[48_000, R44]));
    h.latency = 480;
    h.engine
        .load(
            paths.iter().zip(1..).map(|(p, id)| track(id, p)).collect(),
            0,
            0,
        )
        .expect("load");
    h.settle_polling();
    let played = play_to_end(&mut h, 184_200 - BLOCK);

    // 96 kHz was asked for once, refused, and never asked for again (the fourth track is
    // resampled at once: the stream goes back to 48 kHz for it).
    assert_eq!(rates(&h), [48_000, R44, R96, 48_000, R44, 48_000]);
    let infos = reopened(&h);
    assert_eq!(infos.len(), 4, "one reopen per track");
    let refusals: Vec<_> = infos.iter().filter(|i| i.fallback.is_some()).collect();
    assert_eq!(refusals.len(), 1, "reported once: {infos:?}");
    let note = refusals[0].fallback.as_deref().expect("a reason");
    assert!(note.contains("96000 Hz refused"), "{note}");
    assert!(note.contains("resampling"), "{note}");
    // The device took the engine's rate exclusively, so the mode did not change.
    assert!(infos.iter().all(|i| i.mode == StreamMode::Exclusive));

    // The 44.1 kHz tracks are bit-exact; the 96 kHz ones are the resampler's output at 48 kHz.
    let lens = [R44 as usize, SECOND, R44 as usize, SECOND];
    let mut at = 0;
    for (i, len) in lens.iter().enumerate() {
        let got = &played.audio[2 * at..2 * (at + len)];
        if rates_of[i] == R44 {
            assert!(got == refs[i].as_slice(), "track {} is bit exact", i + 1);
        } else {
            let (l, r) = sweep(RATE, SECOND);
            let ideal = reference(&l, &r, 0);
            let err = max_err(&got[2 * 600..2 * (SECOND - 600)], &ideal.0, &ideal.1, 600);
            assert!(err < 1.0e-3, "track {} resampled: error {err}", i + 1);
        }
        at += len;
    }
    assert_eq!(played.audio.len(), 2 * at);
    let kinds = h.sink.kinds();
    assert_eq!(underruns(&kinds), 0, "underruns");
    assert_paired(&kinds, false);
}

#[test]
fn a_device_that_refuses_exclusive_altogether_falls_back_to_shared_once() {
    let dir = TempDir::new("excl-shared-fallback");
    let paths: Vec<_> = ["a", "b", "c"]
        .iter()
        .map(|n| dir.path(&format!("{n}.wav")))
        .collect();
    for (p, r) in paths.iter().zip([R44, R96, R44]) {
        sweep_file(p, r);
    }
    let mut h = Harness::exclusive(None);
    h.engine
        .load(
            paths.iter().zip(1..).map(|(p, id)| track(id, p)).collect(),
            0,
            0,
        )
        .expect("load");
    h.settle_polling();
    // The endpoint is taken by another application after the first track started: it takes
    // 44.1 kHz only (what is open), no other exclusive stream.
    h.out.set_device_rates(&[]);
    let _played = play_to_end(&mut h, 140_000);

    // 96 kHz refused, then the engine's own rate refused too: shared, resampling, for the rest
    // of the queue. The third track (44.1 kHz) does not try exclusive mode again.
    assert_eq!(rates(&h), [48_000, R44, R96, 48_000]);
    let infos = reopened(&h);
    assert_eq!(infos.len(), 2, "no reopen for the third track: {infos:?}");
    assert_eq!(infos[0].mode, StreamMode::Exclusive);
    assert_eq!(infos[1].mode, StreamMode::Shared);
    let note = infos[1].fallback.as_deref().expect("a reason");
    assert!(note.contains("96000 Hz refused"), "{note}");
    assert!(note.contains("48000 Hz"), "{note}");
    let kinds = h.sink.kinds();
    assert_eq!(
        left_after(&kinds, 3).map(|l| l.1),
        Some(LeaveCause::Finished)
    );
    assert_eq!(underruns(&kinds), 0, "underruns");
}

#[test]
fn a_device_switch_keeps_the_rate_of_the_playing_file_or_falls_back() {
    let dir = TempDir::new("excl-switch");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    let (_, ref_b) = (sweep_file(&a, R44), sweep_file(&b, R96));
    let mut h = Harness::exclusive(None);
    h.engine.load(vec![track(2, &b)], 0, 0).expect("load");
    h.settle_polling();
    let _ = h.render_exact(9_600, 480);

    // The new device takes 96 kHz: reopened at the file's rate at once, resumed in place.
    h.engine.switch_device(Some("dac".into())).expect("switch");
    assert_eq!(
        h.out.opens().last().cloned(),
        Some((Some("dac".into()), R96))
    );
    h.settle_polling();
    assert_eq!(h.engine.position().expect("position").frame, 4_800);
    let rec = h.render_exact(4_800, 480);
    assert!(
        rec == ref_b[2 * 9_600..2 * (9_600 + 4_800)],
        "resumed in the 96 kHz file"
    );
    assert_eq!(
        rates(&h),
        [48_000, R96, R96],
        "one open per stream, no second reopen"
    );

    // This one does not: 96 kHz refused, 48 kHz taken, the track resampled from then on.
    h.out.set_device_rates(&[48_000]);
    h.engine.switch_device(Some("usb".into())).expect("switch");
    h.settle_polling();
    assert_eq!(rates(&h), [48_000, R96, R96, R96, 48_000]);
    let infos = reopened(&h);
    let note = infos
        .last()
        .and_then(|i| i.fallback.clone())
        .expect("a reason");
    assert!(note.contains("96000 Hz refused"), "{note}");
    assert_eq!(infos.last().map(|i| i.mode), Some(StreamMode::Exclusive));
    // 4,800 timeline frames at the first switch and 4,800 output frames (2,400) since.
    assert_eq!(h.engine.position().expect("position").frame, 7_200);
}

#[test]
fn a_new_load_while_the_output_is_being_reopened_wins() {
    let dir = TempDir::new("excl-supersede");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    let (_, ref_b) = (sweep_file(&a, R44), sweep_file(&b, R96));
    let mut h = Harness::exclusive(None);
    // The second load replaces the first before anything was reopened for it.
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    h.engine.load(vec![track(2, &b)], 0, 0).expect("load");
    h.settle_polling();
    let rec = h.render_exact(4_800, 480);
    assert!(rec == ref_b[..2 * 4_800]);
    assert_eq!(
        rates(&h),
        [48_000, R96],
        "nothing was opened for the load that was replaced"
    );
}

#[test]
fn a_rate_change_in_the_playback_speed_composes_with_the_reopen() {
    let dir = TempDir::new("excl-stretch");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    sweep_file(&a, R44);
    sweep_file(&b, R96);
    let mut h = Harness::exclusive(None);
    h.engine.set_rate(1.25).expect("rate");
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle_polling();
    let played = play_to_end(&mut h, 100_000);

    // Both seconds of audio, 1.25 times faster: 0.8 s of output each, at their own rates.
    assert_eq!(rates(&h), [48_000, R44, R96]);
    let expected = (R44 as f64 / 1.25 + R96 as f64 / 1.25) as usize;
    let got = played.audio.len() / 2;
    assert!(
        got.abs_diff(expected) < 4_000,
        "{got} frames, expected about {expected}"
    );
    // The books: the whole run was played at 1.25, and the control side saw both tracks end at
    // their timeline length.
    let stats = h.engine.rate_stats();
    let rate = stats.source_frames() / stats.wall_frames as f64;
    assert!((rate - 1.25).abs() < 0.002, "time-weighted rate {rate}");
    let kinds = h.sink.kinds();
    for item in [1, 2] {
        let (frame, cause) = left_after(&kinds, item).expect("left");
        assert_eq!(cause, LeaveCause::Finished);
        assert!(
            frame.abs_diff(SECOND as u64) <= 16,
            "item {item} left at {frame}"
        );
    }
    assert_paired(&kinds, false);
}

#[test]
fn a_speed_change_during_the_second_file_restarts_inside_its_own_stream() {
    let dir = TempDir::new("excl-stretch-mid");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    sweep_file(&a, R44);
    sweep_file(&b, R96);
    let mut h = Harness::exclusive(None);
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle_polling();
    // Into the 96 kHz file: all of the first and 20,000 frames of the second.
    let target = R44 as usize + 20_000;
    let mut got = 0;
    let mut block = vec![0.0f32; BLOCK * 2];
    while got < target {
        let want = BLOCK.min(target - got);
        let real = h
            .out
            .render(&mut block[..want * 2], 0)
            .expect("stream open");
        got += real;
        h.engine.poll();
        if real < want {
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    h.engine.set_rate(1.5).expect("rate");
    let played = play_to_end(&mut h, 40_000);
    // The flush reopened the file at the audible place without asking for another stream.
    assert_eq!(rates(&h), [48_000, R44, R96]);
    assert!(h.engine.rate_stats().flushes >= 1);
    let kinds = h.sink.kinds();
    let (frame, cause) = left_after(&kinds, 2).expect("left");
    assert_eq!(cause, LeaveCause::Finished);
    assert!(frame.abs_diff(SECOND as u64) <= 16, "left at {frame}");
    // Both files were played through, once: the source frames the renderer advanced add up to
    // their lengths (a restart repeats or drops a few frames of fade, not more), however long the
    // decoder took to apply the new speed.
    let stats = h.engine.rate_stats();
    let advanced = stats.source_frames();
    let lengths = (R44 + R96) as f64;
    assert!(
        (advanced - lengths).abs() < 2_000.0,
        "advanced {advanced} of {lengths} frames"
    );
    assert!(stats.wall_frames as usize <= played.audio.len() / 2 + R44 as usize + 20_000);
    assert_paired(&kinds, false);
}

/// The same pair through the device pump: 24-bit material reaches an `I24In32` device as the
/// file's own integers, no dither, with the pump's format conversion in the path.
#[test]
fn a_24_bit_pair_reaches_the_device_as_the_files_integers() {
    let dir = TempDir::new("excl-device-bytes");
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    let (ref_a, ref_b) = (sweep_file(&a, R44), sweep_file(&b, R96));

    let (output, dev) = FakeDeviceOutput::new(SampleFormat::I24In32, BLOCK);
    dev.set_exclusive(None);
    let sink = Arc::new(CollectingSink::default());
    let mut engine = Engine::start(
        EngineConfig {
            sample_rate: RATE,
            device: None,
            opener: Arc::new(FsOpener),
            sink: sink.clone(),
            clock: Arc::new(ManualClock::new(UtcMillis::new(1_000_000))),
        },
        Box::new(output),
    )
    .expect("engine starts");
    engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    // The device calls only when the decoder has a block ready (a partial block mid-stream would
    // put silence inside the recording), except between two streams and for the last frames.
    let total = (R44 + R96) as usize;
    let mut played = 0;
    for _ in 0..60_000 {
        if engine.command_pending() {
            // The decoder needs the callback to acknowledge the flush the load started.
            dev.service_flush();
        }
        let ready = dev.buffered_frames() >= BLOCK
            || engine.reopen_pending()
            || played + dev.buffered_frames() >= total;
        if ready {
            if let Some(PumpStep::Fed { real, .. }) = dev.step() {
                played += real;
            }
        } else {
            std::thread::sleep(Duration::from_micros(100));
        }
        engine.poll();
        if sink.kinds().contains(&EventKind::QueueEnded) {
            break;
        }
    }
    assert!(
        sink.kinds().contains(&EventKind::QueueEnded),
        "{:?}",
        sink.kinds()
    );
    let streams: Vec<_> = dev
        .streams()
        .into_iter()
        .filter(|s| !s.1.is_empty())
        .collect();
    assert_eq!(streams.len(), 2, "one stream per rate");
    assert_eq!(streams[0].0, R44);
    assert_eq!(streams[1].0, R96);
    for ((_, bytes), reference) in streams.iter().zip([&ref_a, &ref_b]) {
        let got = decode_device_bytes(SampleFormat::I24In32, bytes);
        let (audio, tail) = got.split_at(reference.len());
        assert!(
            audio == reference.as_slice(),
            "the device received other samples than the file holds"
        );
        // What follows is the padding of the last, partial callback.
        assert!(tail.len() < BLOCK * 2 && tail.iter().all(|&s| s == 0.0));
    }
}
