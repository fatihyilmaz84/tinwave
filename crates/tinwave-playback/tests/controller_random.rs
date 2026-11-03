//! A seeded random walk through the controller's whole command surface, with the clock jumping
//! across the 30-minute boundaries, checking the invariants the signals must keep whatever the
//! listener does: sessions nest, every event is inside the open session, plays have at most one
//! outcome, seek kinds agree with their direction, queue edits never produce more signals than the
//! edits the controller accepted, no session claims more minutes than it was audible for, nothing
//! is dropped.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use controller_support::*;
use tinwave_core::{Clock, Rng, SeededRng};
use tinwave_playback::{
    LoadIntent, PlayState, PlaybackError, QueueEdit, QueueEditRequest, QueueEntry,
};
use tinwave_signals::{
    InputSource, PlayOrigin, QueueOrigin, RepeatMode, SignalEvent, SignalKind, SignalPayload, View,
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

/// What the walk observed besides the signals: when the controller said it was playing, and how
/// many queue acts it accepted (each may or may not end up as a signal).
#[derive(Default)]
struct Observed {
    /// `(clock ms, playing)` at the end of every step.
    timeline: Vec<(i64, bool)>,
    accepted_queue_acts: usize,
}

fn sample(rig: &Rig, observed: &mut Observed) {
    observed.timeline.push((
        rig.clock.now().as_i64(),
        rig.ctl.status().state == PlayState::Playing,
    ));
}

fn new_entry(files: &Files, dice: &mut Dice, id: u64) -> QueueEntry {
    let pick = dice.below(files.good.len() as u32) as usize;
    let (path, frames) = &files.good[pick];
    entry(id, path, *frames, QueueOrigin::Album)
}

/// A queue edit built on the item the engine is on (mostly) or on the one the walk believes is
/// current (sometimes stale). Anything the controller accepts is mirrored in `queue`.
fn edit_op(
    (seed, files): (u64, &Files),
    op: u32,
    rig: &mut Rig,
    queue: &mut Vec<QueueEntry>,
    index: usize,
    (dice, next_id): (&mut Dice, &mut u64),
    observed: &mut Observed,
) {
    let input = input(dice);
    if op == 21 {
        *next_id += 1;
        let e = new_entry(files, dice, *next_id);
        rig.ctl
            .queue_later(vec![e.clone()], View::Album, input)
            .expect("queue later");
        queue.push(e);
        observed.accepted_queue_acts += 1;
        return;
    }
    let at = rig
        .ctl
        .position()
        .and_then(|p| queue.iter().position(|e| e.track.id == p.item))
        .filter(|_| dice.below(5) != 0)
        .unwrap_or(index);
    let mut tail: Vec<QueueEntry> = queue[at + 1..].to_vec();
    let edit = match op {
        18 => {
            *next_id += 1;
            let e = new_entry(files, dice, *next_id);
            let id = e.track.id;
            tail.insert(0, e);
            QueueEdit::PlayNext {
                items: vec![id],
                from_view: View::Search,
            }
        }
        19 if !tail.is_empty() => {
            let gone = tail.remove(dice.below(tail.len() as u32) as usize);
            QueueEdit::Remove {
                items: vec![gone.track.id],
            }
        }
        20 if tail.len() >= 2 => {
            tail.swap(0, 1);
            QueueEdit::Reorder {
                item: tail[0].track.id,
                distance: -1,
            }
        }
        _ => return,
    };
    let head = queue[at].track.id;
    match rig.ctl.edit_queue(QueueEditRequest {
        head,
        tail: tail.clone(),
        edit,
        input,
    }) {
        Ok(()) => {
            queue.truncate(at + 1);
            queue.extend(tail);
            observed.accepted_queue_acts += 1;
        }
        // The walk's view of what is playing is a poll old, or nothing is.
        Err(PlaybackError::StaleQueue { .. } | PlaybackError::NothingLoaded) => {}
        Err(other) => panic!("seed {seed}: edit {op}: {other:?}"),
    }
}

fn run(seed: u64, files: &Files, rig: &mut Rig, steps: u32) -> (Vec<String>, Observed) {
    let mut dice = Dice(SeededRng::new(seed));
    let mut trace = Vec::new();
    let mut observed = Observed::default();
    let mut next_id = 0u64;
    let mut queue: Vec<QueueEntry> = Vec::new();
    let mut index = 0usize;
    sample(rig, &mut observed);
    for _ in 0..steps {
        let op = dice.below(24);
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
            22 | 23 => {
                let rate = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5][dice.below(6) as usize];
                rig.ctl.set_rate(rate, input(&mut dice)).expect("rate");
            }
            10 => rig.ctl.stop().expect("stop"),
            11 => rig.ctl.set_repeat_mode(
                [RepeatMode::Off, RepeatMode::All, RepeatMode::One][dice.below(3) as usize],
            ),
            12 => rig.ctl.switch_device(Some("dev".into())).expect("switch"),
            18..=21 if !queue.is_empty() => {
                edit_op(
                    (seed, files),
                    op,
                    rig,
                    &mut queue,
                    index,
                    (&mut dice, &mut next_id),
                    &mut observed,
                );
            }
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
        sample(rig, &mut observed);
    }
    (trace, observed)
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

fn check(seed: u64, signals: &[SignalEvent], trace: &[String], observed: &Observed) {
    let context = || format!("seed {seed}, trace {trace:?}, kinds {:?}", kinds(signals));
    let mut open_session = None;
    let mut started_at = 0i64;
    let mut queue_signals = 0usize;
    let mut open_play: Option<(tinwave_core::RecordingId, bool)> = None;
    let mut last_seq = None;
    for event in signals {
        assert!(last_seq < Some(event.id.seq), "seq order: {}", context());
        last_seq = Some(event.id.seq);
        match event.kind() {
            SignalKind::SessionStarted => {
                assert!(open_session.is_none(), "nested session: {}", context());
                open_session = Some(event.session);
                started_at = event.ts.as_i64();
            }
            SignalKind::SessionEnded => {
                assert_eq!(open_session, Some(event.session), "{}", context());
                open_session = None;
                let SignalPayload::SessionEnded(ended) = event.payload else {
                    panic!("kind");
                };
                // A session never claims more minutes than it was audible for: from its start
                // to the last moment the controller said it was playing in it. (One minute of
                // slack: the walk samples the state once per step, and a step can cross the end
                // of the queue.)
                let ended_at = event.ts.as_i64();
                let last_audible = observed
                    .timeline
                    .iter()
                    .filter(|&&(t, playing)| playing && t >= started_at && t <= ended_at)
                    .map(|&(t, _)| t)
                    .max()
                    .unwrap_or(started_at);
                assert!(
                    i64::from(ended.minutes) * 60_000 <= last_audible - started_at + 60_000,
                    "SessionEnded claims {} min, audible for {} ms: {}",
                    ended.minutes,
                    last_audible - started_at,
                    context()
                );
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
        if matches!(
            event.kind(),
            SignalKind::QueuedNext
                | SignalKind::QueuedLater
                | SignalKind::QueueReordered
                | SignalKind::RemovedFromQueue
        ) {
            queue_signals += 1;
        }
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
            // A gesture that ended where it began is not a change and is not emitted.
            SignalPayload::RateChanged(p) => assert_ne!(p.from, p.to, "{}", context()),
            // A pause is not always followed by a Resumed signal (a stop or a new queue can end
            // it, and a resume with no track current is not a signal), so no pairing is asserted.
            _ => {}
        }
    }
    assert!(open_session.is_none(), "session left open: {}", context());
    // A queue act is recorded at most once, and only if the controller accepted it (a refused
    // edit, an `Auto` one and one outside a session record nothing).
    assert!(
        queue_signals <= observed.accepted_queue_acts,
        "{queue_signals} queue signals for {} accepted acts: {}",
        observed.accepted_queue_acts,
        context()
    );
}

#[test]
fn random_command_sequences_keep_the_signal_invariants() {
    let seeds = std::env::var("TINWAVE_SEEDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(48);
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
    let (mut accepted, mut recorded) = (0, 0);
    for seed in 0..seeds {
        let mut rig = Rig::new(&format!("rnd-{seed}"));
        let (trace, mut observed) = run(seed, &files, &mut rig, 70);
        epilogue(seed, &files, &mut rig);
        sample(&rig, &mut observed);
        let bus = rig.bus.clone();
        let signals = rig.shutdown();
        check(seed, &signals, &trace, &observed);
        plays += signals
            .iter()
            .filter(|e| e.kind() == SignalKind::PlayStarted)
            .count();
        accepted += observed.accepted_queue_acts;
        recorded += signals
            .iter()
            .filter(|e| e.kind() == SignalKind::QueuedNext)
            .count();
        assert_eq!(bus.dropped_total(), 0, "seed {seed}");
    }
    assert!(plays > 0, "the walks never produced a PlayStarted");
    assert!(
        accepted > 0 && recorded > 0,
        "the walks never made a queue edit that was recorded ({accepted} accepted, {recorded} recorded)"
    );
}
