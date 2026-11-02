//! Appending to a queue that has already ended, and what happens around it: buffering is not an
//! underrun, and a device switch never drops an appended track.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use common::*;
use tinwave_playback::{EventKind, QueueItemId, StartCause};

const A_FRAMES: usize = 9_600;

/// Two short WAVs cut from the reference sweep: A is `[0, 9600)`, B is `[9600, 9600 + b_frames)`.
fn files(tag: &str, b_frames: usize) -> (std::path::PathBuf, std::path::PathBuf, TempDir) {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..A_FRAMES], &r[..A_FRAMES]);
    write_wav(
        &b,
        RATE,
        16,
        &l[A_FRAMES..A_FRAMES + b_frames],
        &r[A_FRAMES..A_FRAMES + b_frames],
    );
    (a, b, dir)
}

fn queue_ends(h: &Harness) -> usize {
    h.sink
        .kinds()
        .iter()
        .filter(|k| **k == EventKind::QueueEnded)
        .count()
}

fn underruns(h: &Harness) -> usize {
    h.sink
        .kinds()
        .iter()
        .filter(|k| matches!(k, EventKind::Underrun { .. }))
        .count()
}

fn wait_until_blocked(gate: &Gate) {
    for _ in 0..30_000 {
        if gate.blocked.load(Ordering::Acquire) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    panic!("decoder never reached the gated read");
}

fn plays_b_exactly(h: &mut Harness, b_frames: usize) {
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, ref_r) = reference(&l64, &r64, 16);
    let rec = h.render_exact(b_frames, 480);
    assert_eq!(max_err(&rec, &ref_l, &ref_r, A_FRAMES), 0.0);
}

/// The decoder needs a while to open the appended file. The callbacks that run meanwhile have
/// nothing to play, but that is buffering: the previous track's audio flowing does not make the
/// wait an underrun.
#[test]
fn waiting_for_an_appended_track_to_open_is_not_an_underrun() {
    let (a, b, _dir) = files("append-buffering", 4_800);
    let gate = Arc::new(Gate::default());
    let opener = Arc::new(GatedOpener {
        gate: Arc::clone(&gate),
        allow: 0,
        only: Some(b.clone()),
    });
    let mut h = Harness::with_opener(opener);
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    let _ = h.render_exact(A_FRAMES, 480);
    h.finish();
    // The decoder may still be noticing the end of A while the last callbacks run, which can
    // legitimately count; the baseline is taken once QueueEnded has been seen.
    let baseline = underruns(&h);

    h.engine.append(vec![track(2, &b)]).expect("append");
    wait_until_blocked(&gate);
    // 200 fast callbacks while the decoder is stuck opening B.
    let mut block = vec![0.0f32; 960];
    for _ in 0..200 {
        assert_eq!(h.out.render(&mut block, 0), Some(0));
        h.engine.poll();
    }
    assert_eq!(underruns(&h), baseline, "{:?}", h.sink.kinds());

    gate.open();
    plays_b_exactly(&mut h, 4_800);
    h.run_until("second QueueEnded", |h| queue_ends(h) == 2);
    assert_paired(&h.sink.kinds(), false);
}

/// Directed regression for `Reconfigure{item: None}`: when no track is current, the decoder keeps
/// its position in the queue, which is the first appended track. Here the switch is queued while
/// the decoder is still stuck opening that track, so the engine has nothing to name.
#[test]
fn switch_while_an_appended_track_is_still_opening_does_not_drop_it() {
    let (a, b, _dir) = files("append-switch-opening", 4_800);
    let gate = Arc::new(Gate::default());
    let opener = Arc::new(GatedOpener {
        gate: Arc::clone(&gate),
        allow: 0,
        only: Some(b.clone()),
    });
    let mut h = Harness::with_opener(opener);
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    let _ = h.render_exact(A_FRAMES, 480);
    h.finish();

    h.engine.append(vec![track(2, &b)]).expect("append");
    wait_until_blocked(&gate);
    h.engine
        .switch_device(Some("other".into()))
        .expect("switch");
    gate.open();
    h.settle();

    plays_b_exactly(&mut h, 4_800);
    h.run_until("second QueueEnded", |h| queue_ends(h) == 2);
    let kinds = h.sink.kinds();
    assert_paired(&kinds, false);
    assert!(
        kinds.contains(&EventKind::TrackStarted {
            item: QueueItemId(2),
            frame: 0,
            cause: StartCause::Advance
        }),
        "{kinds:?}"
    );
    assert_eq!(h.out.opens().len(), 2);
}

/// A short appended track that is already decoded into the ring but not yet heard (it is inside
/// the output latency) is neither current nor announced; a switch must still resume it.
#[test]
fn switch_after_an_appended_track_is_decoded_but_not_heard_does_not_drop_it() {
    let (a, b, _dir) = files("append-switch-decoded", 1_000);
    let mut h = Harness::new();
    h.latency = 960;
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    let _ = h.render_exact(A_FRAMES, 480);
    h.finish();

    h.engine.append(vec![track(2, &b)]).expect("append");
    // Fully decoded, nothing rendered yet: B sits in the ring, not yet heard.
    h.wait_buffered(1_000);
    h.engine
        .switch_device(Some("other".into()))
        .expect("switch");
    h.settle();

    plays_b_exactly(&mut h, 1_000);
    h.run_until("second QueueEnded", |h| queue_ends(h) == 2);
    let kinds = h.sink.kinds();
    assert_paired(&kinds, false);
    let starts: Vec<_> = kinds
        .iter()
        .filter(|k| {
            matches!(
                k,
                EventKind::TrackStarted {
                    item: QueueItemId(2),
                    ..
                }
            )
        })
        .collect();
    assert_eq!(starts.len(), 1, "announced exactly once: {kinds:?}");
}
