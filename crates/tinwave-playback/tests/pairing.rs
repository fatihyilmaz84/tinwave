//! The leave-pairing invariant, for the signal layer's sake: every `TrackStarted` is closed by
//! exactly one `TrackLeft` for the same item before the next `TrackStarted`, whatever the user does
//! and whatever the files do. Two review repro scenarios, then a seeded random-sequence test that
//! drives the engine through load / append / seek / stop / pause / device switches with files that
//! fail to open or fail mid-decode, checking the invariant and position monotonicity after every
//! step.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::Arc;

use common::*;
use tinwave_core::{Rng, SeededRng};
use tinwave_playback::{EventKind, PlaybackError, Position, StartCause};

fn wav_pair(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, TempDir) {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);
    (a, b, dir)
}

fn without_noise(h: &Harness) -> Vec<EventKind> {
    h.sink
        .kinds()
        .into_iter()
        .filter(|k| {
            !matches!(
                k,
                EventKind::PositionMark { .. } | EventKind::Underrun { .. }
            )
        })
        .collect()
}

/// Seeking once the queue has ended is rejected: there is no current track to seek in, and
/// restarting one is a `load`, which carries its own `TrackStarted{Requested}`.
#[test]
fn seek_after_the_queue_ended_is_rejected_and_nothing_is_emitted() {
    let (a, b, _dir) = wav_pair("pairing-seek-after-end");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 1, 40_000)
        .expect("load");
    let _ = h.render_exact(TOTAL - SPLIT - 40_000, 480);
    h.finish();
    let before = h.sink.kinds();

    assert_eq!(h.engine.seek(1_000), Err(PlaybackError::NothingLoaded));
    h.engine.poll();
    assert_eq!(h.sink.kinds(), before, "a rejected seek emits nothing");
    assert_paired(&without_noise(&h), false);

    // Replaying is an explicit load, with its own start and end.
    h.engine
        .load(vec![track(2, &b)], 0, 30_000)
        .expect("replay");
    let _ = h.render_exact(TOTAL - SPLIT - 30_000, 480);
    // The first run's QueueEnded is already in the list: wait for the second one.
    h.run_until("second QueueEnded", |h| {
        h.sink
            .kinds()
            .iter()
            .filter(|k| **k == EventKind::QueueEnded)
            .count()
            == 2
    });
    let kinds = without_noise(&h);
    assert_paired(&kinds, false);
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, EventKind::TrackStarted { .. }))
            .count(),
        2
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, EventKind::TrackLeft { .. }))
            .count(),
        2
    );
}

#[test]
fn seek_is_rejected_after_stop_and_before_anything_is_loaded() {
    let (a, _b, _dir) = wav_pair("pairing-seek-nothing");
    let mut h = Harness::new();
    assert_eq!(h.engine.seek(5), Err(PlaybackError::NothingLoaded));
    h.engine.load(vec![track(1, &a)], 0, 0).expect("load");
    let _ = h.render_exact(480, 480);
    h.engine.stop().expect("stop");
    assert_eq!(h.engine.seek(5), Err(PlaybackError::NothingLoaded));
    assert_paired(&h.sink.kinds(), false);
}

/// A failed `switch_device` has already sent the Reconfigure, so the decoder restarts the stream;
/// the engine must adopt that restart silently instead of announcing the same track again.
#[test]
fn failed_switch_device_does_not_start_the_track_twice() {
    let (a, b, _dir) = wav_pair("pairing-failed-switch");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(4_800, 480);

    h.out.fail_next_open();
    let err = h
        .engine
        .switch_device(Some("gone".into()))
        .expect_err("refused");
    assert!(matches!(err, PlaybackError::Output(_)));
    for _ in 0..200 {
        h.engine.poll();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let started = h
        .sink
        .kinds()
        .iter()
        .filter(|k| matches!(k, EventKind::TrackStarted { .. }))
        .count();
    assert_eq!(started, 1, "{:?}", without_noise(&h));

    h.engine.switch_device(None).expect("retry");
    h.settle();
    let rec = h.render_exact(TOTAL - 4_800, 480);
    h.finish();
    let (l64, r64) = sweep(RATE, TOTAL);
    let (ref_l, ref_r) = reference(&l64, &r64, 16);
    assert_eq!(
        max_err(&rec, &ref_l, &ref_r, 4_800),
        0.0,
        "resumed at the exact frame"
    );
    let kinds = without_noise(&h);
    assert_paired(&kinds, false);
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(
                k,
                EventKind::TrackStarted {
                    cause: StartCause::Advance,
                    ..
                }
            ))
            .count(),
        1,
        "only the hand-off from A to B is an advance: {kinds:?}"
    );
}

// ---- seeded random sequences ---------------------------------------------------------------

struct Fixtures {
    _dir: TempDir,
    good: [std::path::PathBuf; 3],
    /// A valid WAV that `BreakingOpener` cuts off mid-stream.
    broken: std::path::PathBuf,
    garbage: std::path::PathBuf,
    missing: std::path::PathBuf,
}

fn fixtures() -> Fixtures {
    let dir = TempDir::new("pairing-property");
    let (l, r) = sweep(RATE, TOTAL);
    let make = |name: &str, frames: usize| {
        let p = dir.path(name);
        write_wav(&p, RATE, 16, &l[..frames], &r[..frames]);
        p
    };
    let good = [
        make("g0.wav", 9_600),
        make("g1.wav", 14_400),
        make("g2.wav", 24_000),
    ];
    let broken = make("broken.wav", 30_000);
    let garbage = dir.path("garbage.wav");
    std::fs::write(&garbage, vec![0x55u8; 2_048]).expect("write");
    let missing = dir.path("missing.wav");
    Fixtures {
        _dir: dir,
        good,
        broken,
        garbage,
        missing,
    }
}

struct Dice(SeededRng);

impl Dice {
    fn below(&mut self, n: u32) -> u32 {
        let mut bytes = [0u8; 4];
        self.0.fill_bytes(&mut bytes).expect("seeded rng");
        u32::from_le_bytes(bytes) % n
    }
}

fn random_track(
    fx: &Fixtures,
    dice: &mut Dice,
    ids: &mut u64,
) -> (tinwave_playback::Track, &'static str) {
    *ids += 1;
    let (path, kind) = match dice.below(10) {
        0 => (fx.missing.clone(), "missing"),
        1 => (fx.garbage.clone(), "garbage"),
        2 | 3 => (fx.broken.clone(), "broken"),
        n => (fx.good[(n % 3) as usize].clone(), "good"),
    };
    (track(*ids, path), kind)
}

/// One random sequence. Returns the first violation with the seed and the trace of operations.
fn run_sequence(seed: u64, fx: &Fixtures, steps: usize) {
    let mut dice = Dice(SeededRng::new(seed));
    let opener = Arc::new(BreakingOpener {
        broken: fx.broken.clone(),
        fail_after: 20_000,
    });
    let mut h = Harness::with_opener(opener);
    let mut ids = 0u64;
    let mut trace: Vec<String> = Vec::new();
    let mut needs_retry = false;
    let mut last_pos: Option<Position> = None;
    let mut paused = false;

    let check = |h: &Harness, trace: &[String]| {
        let kinds = h.sink.kinds();
        let result = std::panic::catch_unwind(|| assert_paired(&kinds, true));
        if result.is_err() {
            panic!("seed {seed}: pairing violated after {trace:?}");
        }
    };

    for step in 0..steps {
        let op = if needs_retry { 99 } else { dice.below(24) };
        let mut command = false;
        match op {
            0..=3 => {
                let n = 1 + dice.below(4);
                let queue: Vec<_> = (0..n)
                    .map(|_| random_track(fx, &mut dice, &mut ids))
                    .collect();
                let index = dice.below(n) as usize;
                let start = if dice.below(3) == 0 {
                    u64::from(dice.below(3_000))
                } else {
                    0
                };
                trace.push(format!(
                    "load {:?} index {index} start {start}",
                    queue.iter().map(|(t, k)| (t.id.0, *k)).collect::<Vec<_>>()
                ));
                h.engine
                    .load(queue.into_iter().map(|(t, _)| t).collect(), index, start)
                    .expect("load");
                command = true;
            }
            4 | 5 => {
                let n = 1 + dice.below(2);
                let more: Vec<_> = (0..n)
                    .map(|_| random_track(fx, &mut dice, &mut ids))
                    .collect();
                trace.push(format!(
                    "append {:?}",
                    more.iter().map(|(t, k)| (t.id.0, *k)).collect::<Vec<_>>()
                ));
                h.engine
                    .append(more.into_iter().map(|(t, _)| t).collect())
                    .expect("append");
            }
            6..=8 => {
                let frame = u64::from(dice.below(15_000));
                trace.push(format!("seek {frame}"));
                match h.engine.seek(frame) {
                    Ok(()) => {}
                    Err(PlaybackError::NothingLoaded) => trace.push("  (nothing loaded)".into()),
                    Err(other) => panic!("seed {seed}: seek failed unexpectedly: {other:?}"),
                }
                command = true;
            }
            9 => {
                trace.push("stop".into());
                h.engine.stop().expect("stop");
                command = true;
            }
            10 => {
                trace.push("pause".into());
                h.engine.pause();
                paused = true;
            }
            11 => {
                trace.push("resume".into());
                h.engine.resume();
                paused = false;
            }
            12 => {
                trace.push("switch ok".into());
                h.engine.switch_device(Some("dev".into())).expect("switch");
                command = true;
            }
            13 => {
                trace.push("switch refused".into());
                h.out.fail_next_open();
                h.engine
                    .switch_device(Some("gone".into()))
                    .expect_err("refused");
                for _ in 0..20 {
                    h.engine.poll();
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                needs_retry = true;
                last_pos = None;
            }
            99 => {
                trace.push("switch retry".into());
                h.engine.switch_device(None).expect("retry");
                needs_retry = false;
                command = true;
            }
            14..=19 => {
                // Play some callbacks without waiting for the decoder beyond a short spin.
                let blocks = 1 + dice.below(30);
                trace.push(format!("play {blocks} blocks"));
                let mut block = vec![0.0f32; 960];
                let mut empty = 0;
                for _ in 0..blocks {
                    for _ in 0..6 {
                        if h.out.buffered_frames().unwrap_or(480) >= 480 {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_micros(100));
                    }
                    let real = h.out.render(&mut block, 0).unwrap_or(0);
                    h.engine.poll();
                    check(&h, &trace);
                    // Nothing to play (stopped, paused, ended): the rest of the blocks add nothing.
                    empty = if real == 0 { empty + 1 } else { 0 };
                    if empty >= 3 {
                        break;
                    }
                }
            }
            _ => {
                trace.push("play to the end".into());
                if paused {
                    h.engine.resume();
                    paused = false;
                }
                let ended_before = h
                    .sink
                    .kinds()
                    .iter()
                    .filter(|k| **k == EventKind::QueueEnded)
                    .count();
                let mut block = vec![0.0f32; 960];
                let mut idle = 0;
                for _ in 0..3_000 {
                    let real = h.out.render(&mut block, 0).unwrap_or(0);
                    h.engine.poll();
                    let ended_now = h
                        .sink
                        .kinds()
                        .iter()
                        .filter(|k| **k == EventKind::QueueEnded)
                        .count();
                    if ended_now > ended_before {
                        break;
                    }
                    if real < 480 {
                        // Nothing playing (stopped, already ended): do not wait for an end that
                        // cannot come.
                        idle += 1;
                        if idle > 60 {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_micros(100));
                    } else {
                        idle = 0;
                    }
                }
            }
        }
        if command {
            h.settle();
            last_pos = None;
        }
        h.engine.poll();
        check(&h, &trace);

        // Position never goes backwards while the same track plays without a command in between.
        if let Some(now) = h.engine.position() {
            if let Some(before) = last_pos {
                if before.item == now.item {
                    assert!(
                        now.frame >= before.frame,
                        "seed {seed} step {step}: position went back {before:?} -> {now:?} after {trace:?}"
                    );
                }
            }
            last_pos = Some(now);
        } else {
            last_pos = None;
        }
    }

    // Wind down: restore the output, stop, and every started track must be closed.
    if needs_retry {
        h.engine.switch_device(None).expect("final retry");
        h.settle();
    }
    h.engine.stop().expect("final stop");
    h.settle();
    h.engine.poll();
    let kinds = h.sink.kinds();
    let result = std::panic::catch_unwind(|| assert_paired(&kinds, false));
    if result.is_err() {
        panic!("seed {seed}: unclosed track at the end after {trace:?}");
    }
}

#[test]
fn random_command_and_failure_sequences_keep_every_start_paired_with_one_leave() {
    let fx = fixtures();
    let seeds = std::env::var("TINWAVE_PAIRING_SEEDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(32);
    for seed in 1..=seeds {
        run_sequence(seed, &fx, 36);
    }
}
