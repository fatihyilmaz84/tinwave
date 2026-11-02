//! No decoder or file IO on the render thread: a reader that blocks forever must not stall the
//! callback or any control call, and playback resumes when the reader does.
//!
//! The source blocks inside `Read::read`, on the decoder thread. If anything on the callback or
//! control path waited for the decoder, the watchdog below would fire instead of the test
//! finishing.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use tinwave_playback::EventKind;

/// Run `body` on a thread; if it does not finish, release the gate (so the test can end) and fail.
fn with_watchdog(gate: &Arc<Gate>, body: impl FnOnce() + Send + 'static) {
    let (done_tx, done_rx) = channel();
    let worker = std::thread::spawn(move || {
        body();
        let _ = done_tx.send(());
    });
    if done_rx.recv_timeout(Duration::from_secs(20)).is_err() {
        gate.open();
        panic!("render callback or a control call waited for the blocked reader");
    }
    worker.join().expect("test body panicked");
}

/// Keeps running callbacks (they acknowledge the flush the load started) until the decoder thread
/// is stuck inside the gated `read`.
fn wait_until_decoder_is_blocked(h: &Harness, gate: &Gate) {
    let mut block = vec![0.0f32; 960];
    for _ in 0..30_000 {
        if gate.blocked.load(Ordering::Acquire) {
            return;
        }
        let _ = h.out.render(&mut block, 0);
        std::thread::sleep(Duration::from_micros(100));
    }
    panic!("decoder never reached the blocking read");
}

#[test]
fn blocked_reader_does_not_stall_callback_or_control_calls() {
    let dir = TempDir::new("blocking-reader");
    let (l, r) = sweep(RATE, TOTAL);
    let path = dir.path("long.wav");
    write_wav(&path, RATE, 16, &l, &r);

    let gate = Arc::new(Gate::default());
    // The decoder gets through the header only, then blocks before any audio is decoded.
    let opener = Arc::new(GatedOpener {
        gate: Arc::clone(&gate),
        allow: 100,
        only: None,
    });
    let gate_in_body = Arc::clone(&gate);
    with_watchdog(&gate, move || {
        let gate = gate_in_body;
        let mut h = Harness::with_opener(opener);
        h.engine
            .load(vec![track(1, &path)], 0, 0)
            .expect("load returns immediately");
        wait_until_decoder_is_blocked(&h, &gate);

        // 2000 callbacks (20 s of audio time) against a decoder that is stuck in `read`:
        // each returns at once with silence.
        let mut block = vec![1.0f32; 960];
        for _ in 0..2_000 {
            let real = h.out.render(&mut block, 0).expect("open");
            assert_eq!(real, 0);
            assert!(block.iter().all(|&s| s == 0.0));
        }
        // Control calls are non-blocking too.
        h.engine.set_volume(0.5).expect("volume");
        h.engine.pause();
        h.engine.resume();
        let _ = h.engine.position();
        h.engine
            .seek(1_000)
            .expect("seek is queued, not waited for");
        h.engine.poll();
        for _ in 0..50 {
            h.out.render(&mut block, 0).expect("open");
        }

        // Release the reader: the queued seek completes and audio flows again.
        gate.open();
        let rec = h.render_exact(4_800, 480);
        let (l64, r64) = sweep(RATE, TOTAL);
        let refs = reference(&l64, &r64, 16);
        // Volume was set to 0.5 above.
        let mut worst = 0.0f32;
        for (i, f) in rec.chunks_exact(2).enumerate() {
            worst = worst.max((f[0] - 0.5 * refs.0[1_000 + i]).abs());
        }
        assert!(worst < 1e-6, "audio after unblocking is wrong: {worst}");
    });
}

#[test]
fn reader_that_stalls_mid_track_causes_underruns_not_a_stalled_callback() {
    let dir = TempDir::new("blocking-reader-mid");
    let (l, r) = sweep(RATE, TOTAL * 3);
    let path = dir.path("long.wav");
    write_wav(&path, RATE, 16, &l, &r);

    let gate = Arc::new(Gate::default());
    // About 0.5 s of audio (16-bit stereo) gets through, then the reader blocks.
    let opener = Arc::new(GatedOpener {
        gate: Arc::clone(&gate),
        allow: 100_000,
        only: None,
    });
    let gate_in_body = Arc::clone(&gate);
    with_watchdog(&gate, move || {
        let gate = gate_in_body;
        let mut h = Harness::with_opener(opener);
        h.engine.load(vec![track(1, &path)], 0, 0).expect("load");
        // The flush acknowledgement needs a few callbacks; after that nothing is rendered until the
        // decoder has filled the ring up to the stall.
        h.settle();
        for _ in 0..30_000 {
            if gate.blocked.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_micros(100));
        }
        assert!(
            gate.blocked.load(Ordering::Acquire),
            "decoder never reached the stall"
        );

        let mut heard = 0usize;
        let mut starved = 0usize;
        let mut block = vec![0.0f32; 960];
        for _ in 0..3_000 {
            let real = h.out.render(&mut block, 0).expect("open");
            heard += real;
            starved += usize::from(real < 480);
            h.engine.poll();
        }
        assert!(
            heard > 10_000,
            "audio decoded before the stall is played ({heard} frames)"
        );
        assert!(
            heard < 48_000,
            "nothing beyond the stall point can be played ({heard} frames)"
        );
        assert!(starved > 2_000, "starved callbacks are silence ({starved})");
        assert!(
            h.sink
                .kinds()
                .iter()
                .any(|k| matches!(k, EventKind::Underrun { .. })),
            "the starvation is reported as an Underrun event"
        );
        gate.open();
    });
}
