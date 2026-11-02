//! Listening sessions (catalog section 8) on the injected clock: a session starts with the first
//! play after 30 minutes of silence and ends after 30 minutes of silence, to the millisecond.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use controller_support::*;
use tinwave_core::TzOffsetMin;
use tinwave_playback::{LoadIntent, PlayState};
use tinwave_signals::{InputSource, PlayOrigin, QueueOrigin, SignalKind, SignalPayload};

const SECOND: usize = 48_000;
const MINUTE_MS: i64 = 60_000;
const HALF_HOUR_MS: i64 = 30 * MINUTE_MS;

/// A rig with one long track playing, paused two minutes after the start. Returns the rig and the
/// clock reading at the pause.
fn paused_after_two_minutes(tag: &str) -> (Rig, i64) {
    let mut rig = Rig::new(tag);
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Playlist, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.clock.advance(2 * MINUTE_MS);
    rig.ctl.pause(InputSource::Mouse);
    let paused_at = rig.clock.now_ms();
    rig.new_signals();
    (rig, paused_at)
}

trait ClockMs {
    fn now_ms(&self) -> i64;
}

impl ClockMs for tinwave_core::ManualClock {
    fn now_ms(&self) -> i64 {
        tinwave_core::Clock::now(self).as_i64()
    }
}

#[test]
fn the_first_play_starts_a_session_with_its_origin_and_the_timezone() {
    let mut rig = Rig::new("ses-start");
    let a = rig.wav("a.wav", SECOND);
    let e = rig.entry(1, &a, SECOND);
    rig.select(vec![e], 0, PlayOrigin::Radio, InputSource::Mouse);

    let signals = rig.signals();
    let started = only(&signals, SignalKind::SessionStarted);
    let SignalPayload::SessionStarted(p) = started.payload else {
        panic!("kind");
    };
    assert_eq!(p.start_origin, PlayOrigin::Radio);
    assert_eq!(started.session, rig.first_session_id());
    assert_eq!(started.recording, None);
    assert_eq!(rig.ctl.status().session, Some(rig.first_session_id()));
    assert_eq!(
        *rig.tz.started.lock().expect("lock"),
        [TzOffsetMin::new(120).expect("offset")],
        "the shell is told the offset once, when the session starts"
    );
}

#[test]
fn a_second_play_inside_the_session_does_not_start_another() {
    let mut rig = Rig::new("ses-continue");
    let a = rig.wav("a.wav", SECOND);
    let b = rig.wav("b.wav", SECOND);
    let entries = vec![
        entry(1, &a, SECOND, QueueOrigin::Manual),
        entry(2, &b, SECOND, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();
    rig.clock.advance(HALF_HOUR_MS - 1_000);
    rig.ctl.poll();
    rig.select(entries, 1, PlayOrigin::Search, InputSource::Mouse);

    let signals = rig.signals();
    assert_eq!(
        signals
            .iter()
            .filter(|e| e.kind() == SignalKind::SessionStarted)
            .count(),
        1
    );
    let sessions: std::collections::HashSet<_> = signals.iter().map(|e| e.session).collect();
    assert_eq!(sessions.len(), 1);
    assert_eq!(rig.tz.started.lock().expect("lock").len(), 1);
}

#[test]
fn silence_of_29_59_keeps_the_session_and_30_00_ends_it() {
    let (mut rig, paused_at) = paused_after_two_minutes("ses-boundary");
    let session = rig.first_session_id();

    rig.clock.advance(HALF_HOUR_MS - 1_000);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty(), "29:59 of silence");
    assert_eq!(rig.ctl.status().session, Some(session));

    rig.clock.advance(1_000);
    rig.ctl.poll();
    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::SessionEnded],
        "30:00 of silence"
    );
    let SignalPayload::SessionEnded(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!((p.tracks, p.minutes), (1, 2), "two minutes of listening");
    assert_eq!(fresh[0].session, session);
    assert_eq!(fresh[0].ts.as_i64(), paused_at + HALF_HOUR_MS);
    let status = rig.ctl.status();
    assert_eq!(status.session, None);
    assert_eq!(status.state, PlayState::Paused, "the track is still there");
}

#[test]
fn resuming_after_29_59_continues_the_session() {
    let (mut rig, _) = paused_after_two_minutes("ses-resume-in-time");
    let session = rig.first_session_id();
    rig.clock.advance(HALF_HOUR_MS - 1_000);
    rig.ctl.resume(InputSource::Keyboard);

    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::Resumed]);
    assert_eq!(fresh[0].session, session);
    assert_eq!(rig.ctl.status().session, Some(session));
}

#[test]
fn resuming_after_exactly_30_minutes_is_the_first_play_of_a_new_session() {
    let (mut rig, paused_at) = paused_after_two_minutes("ses-resume-late");
    let first = rig.first_session_id();
    rig.clock.advance(HALF_HOUR_MS);
    rig.ctl.resume(InputSource::Keyboard);

    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [
            SignalKind::SessionEnded,
            SignalKind::SessionStarted,
            SignalKind::Resumed
        ]
    );
    assert_eq!(fresh[0].session, first);
    assert_eq!(fresh[0].ts.as_i64(), paused_at + HALF_HOUR_MS);
    let SignalPayload::SessionStarted(p) = fresh[1].payload else {
        panic!("kind");
    };
    assert_eq!(p.start_origin, PlayOrigin::Resume);
    assert_ne!(fresh[1].session, first);
    assert_eq!(
        fresh[2].session, fresh[1].session,
        "the resume is in the new session"
    );
    assert_eq!(rig.ctl.status().session, Some(fresh[1].session));
    assert_eq!(rig.tz.started.lock().expect("lock").len(), 2);
}

#[test]
fn the_end_of_the_queue_starts_the_silence() {
    let mut rig = Rig::new("ses-queue-end");
    let a = rig.wav("a.wav", SECOND / 2);
    let e = rig.entry(1, &a, SECOND / 2);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();
    rig.new_signals();

    rig.clock.advance(HALF_HOUR_MS - 1);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty());
    rig.clock.advance(1);
    rig.ctl.poll();
    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::SessionEnded]);
    let SignalPayload::SessionEnded(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!((p.tracks, p.minutes), (1, 0));
}

#[test]
fn pause_then_quit_ends_the_session_at_the_quit() {
    let (rig, paused_at) = paused_after_two_minutes("ses-quit");
    let first = rig.first_session_id();
    rig.clock.advance(5 * MINUTE_MS);
    let signals = rig.shutdown();
    let ended = only(&signals, SignalKind::SessionEnded);
    assert_eq!(ended.session, first);
    assert_eq!(ended.ts.as_i64(), paused_at + 5 * MINUTE_MS);
    let SignalPayload::SessionEnded(p) = ended.payload else {
        panic!("kind");
    };
    assert_eq!(
        (p.tracks, p.minutes),
        (1, 2),
        "the 5 paused minutes are not listening"
    );
}

#[test]
fn a_play_that_outlived_its_session_leaves_no_outcome_and_the_next_play_starts_a_new_session() {
    let (mut rig, _) = paused_after_two_minutes("ses-stale-skip");
    let a = rig.wav("b.wav", SECOND);
    let b = rig.entry(2, &a, SECOND);
    rig.clock.advance(HALF_HOUR_MS + MINUTE_MS);
    rig.ctl.poll();
    rig.new_signals();

    rig.load_with(
        vec![b],
        0,
        LoadIntent::Select(PlayOrigin::Search),
        InputSource::Mouse,
    )
    .expect("load");
    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::SessionStarted, SignalKind::PlayStarted],
        "the old, paused play is not scored as a skip"
    );
    assert_ne!(fresh[0].session, rig.first_session_id());
}

#[test]
fn a_volume_change_after_the_session_ended_is_not_recorded() {
    let (mut rig, _) = paused_after_two_minutes("ses-volume-after");
    rig.clock.advance(HALF_HOUR_MS);
    rig.ctl.poll();
    rig.new_signals();
    rig.ctl.set_volume(0.2, InputSource::Mouse).expect("volume");
    rig.clock.advance(1_000);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty());
}

#[test]
fn a_paused_session_counts_as_live_until_it_ends() {
    let (mut rig, _) = paused_after_two_minutes("ses-live");
    let status = rig.ctl.status();
    assert_eq!(status.state, PlayState::Paused);
    assert_eq!(status.recording, Some(recording(1)));
    assert_eq!(status.source, Some(source(1)));
    assert!(status.is_live());
    rig.clock.advance(HALF_HOUR_MS);
    rig.ctl.poll();
    assert!(!rig.ctl.status().is_live(), "30 minutes of pause ended it");
}

#[test]
fn counts_of_tracks_and_minutes_cover_the_whole_session() {
    let mut rig = Rig::new("ses-counts");
    let a = rig.wav("a.wav", SECOND / 4);
    let b = rig.wav("b.wav", SECOND / 4);
    let c = rig.wav("c.wav", SECOND / 4);
    let entries = vec![
        entry(1, &a, SECOND / 4, QueueOrigin::Manual),
        entry(2, &b, SECOND / 4, QueueOrigin::Manual),
        entry(3, &c, SECOND / 4, QueueOrigin::Manual),
    ];
    rig.select(entries, 0, PlayOrigin::Album, InputSource::Mouse);
    rig.clock.advance(7 * MINUTE_MS + 30_000);
    rig.play_to_end();
    rig.clock.advance(HALF_HOUR_MS);
    rig.ctl.poll();

    let ended = only(&rig.signals(), SignalKind::SessionEnded);
    let SignalPayload::SessionEnded(p) = ended.payload else {
        panic!("kind");
    };
    assert_eq!((p.tracks, p.minutes), (3, 7));
}
