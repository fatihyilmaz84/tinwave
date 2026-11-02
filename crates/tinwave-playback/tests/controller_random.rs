//! A seeded random walk through the controller's whole command surface, with the clock jumping
//! across the 30-minute boundaries, checking the invariants the signals must keep whatever the
//! listener does: sessions nest, every event is inside the open session, plays have at most one
//! outcome, seek kinds agree with their direction, nothing is dropped.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use controller_support::*;
use tinwave_core::{Rng, SeededRng};
use tinwave_playback::{LoadIntent, PlaybackError, QueueEntry};
use tinwave_signals::{
    InputSource, PlayOrigin, QueueOrigin, RepeatMode, SignalEvent, SignalKind, SignalPayload,
};

const SECOND: usize = 48_000;

struct Dice(SeededRng);

impl Dice {
    fn below(&mut self, n: u32) -> u32 {
        let mut bytes = [0u8; 4];
        self.0.fill_bytes(&mut bytes).expect("seeded rng");
        u32::from_le_bytes(bytes) % n
    }
}

struct Files {
    good: Vec<(std::path::PathBuf, usize)>,
    missing: std::path::PathBuf,
}

fn input(dice: &mut Dice) -> InputSource {
    [
        InputSource::Mouse,
        InputSource::Keyboard,
        InputSource::MediaKey,
        InputSource::Tray,
    ][dice.below(4) as usize]
}

fn play_for(rig: &mut Rig, callbacks: u32) {
    let mut block = vec![0.0f32; 480 * 2];
    for _ in 0..callbacks {
        if rig.out.buffered_frames().unwrap_or(0) >= 480 {
            let _ = rig.out.render(&mut block, 0);
        } else {
            std::thread::sleep(std::time::Duration::from_micros(150));
        }
        rig.ctl.poll();
    }
}

fn run(seed: u64, files: &Files, rig: &mut Rig, steps: u32) -> Vec<String> {
    let mut dice = Dice(SeededRng::new(seed));
    let mut trace = Vec::new();
    let mut next_id = 0u64;
    let mut queue: Vec<QueueEntry> = Vec::new();
    let mut index = 0usize;
    for _ in 0..steps {
        let op = dice.below(18);
        trace.push(format!("op {op}"));
        match op {
            0..=2 => {
                let n = 1 + dice.below(3) as usize;
                queue = (0..n)
                    .map(|_| {
                        next_id += 1;
                        if dice.below(6) == 0 {
                            entry(next_id, &files.missing, SECOND, QueueOrigin::Radio)
                        } else {
                            let pick = dice.below(files.good.len() as u32) as usize;
                            let (path, frames) = &files.good[pick];
                            entry(next_id, path, *frames, QueueOrigin::Album)
                        }
                    })
                    .collect();
                index = dice.below(n as u32) as usize;
                let origin = [PlayOrigin::Search, PlayOrigin::Album, PlayOrigin::Radio]
                    [dice.below(3) as usize];
                let i = input(&mut dice);
                rig.load_with(queue.clone(), index, LoadIntent::Select(origin), i)
                    .expect("load");
            }
            3 | 4 if !queue.is_empty() => {
                let forward = op == 3;
                index = if forward {
                    (index + 1) % queue.len()
                } else {
                    (index + queue.len() - 1) % queue.len()
                };
                let intent = if forward {
                    LoadIntent::Next
                } else {
                    LoadIntent::Previous
                };
                let i = input(&mut dice);
                rig.load_with(queue.clone(), index, intent, i)
                    .expect("next/previous");
            }
            5 => rig.ctl.pause(input(&mut dice)),
            6 => rig.ctl.resume(input(&mut dice)),
            7 | 8 => {
                let ms = u64::from(dice.below(15_000));
                match rig.ctl.seek_ms(ms, input(&mut dice)) {
                    Ok(()) | Err(PlaybackError::NothingLoaded) => {}
                    Err(other) => panic!("seed {seed}: seek: {other:?}"),
                }
            }
            9 => {
                let v = dice.below(101) as f32 / 100.0;
                rig.ctl.set_volume(v, input(&mut dice)).expect("volume");
            }
            10 => rig.ctl.stop().expect("stop"),
            11 => rig.ctl.set_repeat_mode(
                [RepeatMode::Off, RepeatMode::All, RepeatMode::One][dice.below(3) as usize],
            ),
            12 => rig.ctl.switch_device(Some("dev".into())).expect("switch"),
            13 => {
                // A pause of 20..40 minutes: the 30-minute boundary falls inside the range.
                rig.clock.advance(i64::from(20 + dice.below(21)) * 60_000);
                rig.ctl.poll();
            }
            _ => {
                rig.clock.advance(i64::from(dice.below(1_500)));
                play_for(rig, 1 + dice.below(40));
            }
        }
        rig.ctl.poll();
    }
    trace
}

/// Whatever the walk left behind, a track played from a clean start is heard once its first 100 ms
/// have been rendered, and starts exactly once however long it then plays.
fn epilogue(seed: u64, files: &Files, rig: &mut Rig) {
    rig.ctl.resume(InputSource::Mouse);
    rig.ctl.stop().expect("stop");
    let (path, frames) = &files.good[2];
    let e = entry(1_000, path, *frames, QueueOrigin::Album);
    let before = rig.of_kind(SignalKind::PlayStarted).len();
    rig.select(vec![e], 0, PlayOrigin::Search, InputSource::Mouse);
    rig.settle();
    rig.hear();
    let started = rig.of_kind(SignalKind::PlayStarted);
    assert_eq!(
        started.len(),
        before + 1,
        "seed {seed}: a heard play starts exactly once"
    );
    assert_eq!(started[before].recording, Some(recording(1_000)));
    rig.render_exact(SECOND, 480);
    assert_eq!(
        rig.of_kind(SignalKind::PlayStarted).len(),
        before + 1,
        "seed {seed}: playing on does not start it again"
    );
}

fn check(seed: u64, signals: &[SignalEvent], trace: &[String]) {
    let context = || format!("seed {seed}, trace {trace:?}, kinds {:?}", kinds(signals));
    let mut open_session = None;
    let mut open_play: Option<(tinwave_core::RecordingId, bool)> = None;
    let mut last_seq = None;
    for event in signals {
        assert!(last_seq < Some(event.id.seq), "seq order: {}", context());
        last_seq = Some(event.id.seq);
        match event.kind() {
            SignalKind::SessionStarted => {
                assert!(open_session.is_none(), "nested session: {}", context());
                open_session = Some(event.session);
            }
            SignalKind::SessionEnded => {
                assert_eq!(open_session, Some(event.session), "{}", context());
                open_session = None;
            }
            other => {
                assert_eq!(
                    open_session,
                    Some(event.session),
                    "{other}: outside its session: {}",
                    context()
                );
            }
        }
        assert!(
            event.ts >= event.session.created_at(),
            "ts before session start: {}",
            context()
        );
        match event.payload {
            SignalPayload::PlayStarted(_) => open_play = event.recording.map(|r| (r, false)),
            SignalPayload::Completed(_) | SignalPayload::Skipped(_) => {
                let (recording, done) = open_play.expect("outcome without a play");
                assert_eq!(Some(recording), event.recording, "{}", context());
                assert!(!done, "two outcomes for one play: {}", context());
                open_play = Some((recording, true));
            }
            SignalPayload::SeekForward(p) => {
                assert!(p.to_ds.ds() > p.from_ds.ds(), "{}", context());
            }
            SignalPayload::SeekBack(p) => {
                assert!(p.to_ds.ds() < p.from_ds.ds(), "{}", context());
            }
            // A pause is not always followed by a Resumed signal (a stop or a new queue can end
            // it, and a resume with no track current is not a signal), so no pairing is asserted.
            _ => {}
        }
    }
    assert!(open_session.is_none(), "session left open: {}", context());
}

#[test]
fn random_command_sequences_keep_the_signal_invariants() {
    let seeds = std::env::var("TINWAVE_SEEDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(24);
    let probe = Rig::new("rnd-files");
    let good = vec![
        (probe.wav("g1.wav", SECOND), SECOND),
        (probe.wav("g2.wav", 3 * SECOND), 3 * SECOND),
        (probe.wav("g3.wav", 20 * SECOND), 20 * SECOND),
    ];
    let files = Files {
        missing: probe.dir.path("missing.wav"),
        good,
    };
    let mut plays = 0;
    for seed in 0..seeds {
        let mut rig = Rig::new(&format!("rnd-{seed}"));
        let trace = run(seed, &files, &mut rig, 70);
        epilogue(seed, &files, &mut rig);
        let bus = rig.bus.clone();
        let signals = rig.shutdown();
        check(seed, &signals, &trace);
        plays += signals
            .iter()
            .filter(|e| e.kind() == SignalKind::PlayStarted)
            .count();
        assert_eq!(bus.dropped_total(), 0, "seed {seed}");
    }
    assert!(plays > 0, "the walks never produced a PlayStarted");
}
