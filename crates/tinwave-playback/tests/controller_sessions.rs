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
    assert_eq!(
        rig.ctl.status().session,
        None,
        "a session is the listener's time: it begins when audio is heard"
    );
    rig.hear();

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
    rig.hear();

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

/// The paused play of `paused_after_two_minutes` is left by "next" after `silence_ms` of pause.
fn next_after_pause(tag: &str, silence_ms: i64) -> (Rig, Vec<tinwave_signals::SignalEvent>) {
    let (mut rig, _) = paused_after_two_minutes(tag);
    let b = rig.wav("b.wav", 10 * SECOND);
    let a = rig.dir.path("a.wav");
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 10 * SECOND, QueueOrigin::Playlist),
    ];
    rig.clock.advance(silence_ms);
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::MediaKey)
        .expect("next");
    let fresh = rig.new_signals();
    (rig, fresh)
}

#[test]
fn next_after_exactly_30_minutes_of_pause_scores_the_skip_in_the_new_session() {
    let (mut rig, fresh) = next_after_pause("ses-stale-skip", HALF_HOUR_MS);
    assert_eq!(
        kinds(&fresh),
        [
            SignalKind::SessionEnded,
            SignalKind::SessionStarted,
            SignalKind::Skipped
        ],
        "the old play's outcome is not dropped"
    );
    let SignalPayload::SessionStarted(started) = fresh[1].payload else {
        panic!("kind");
    };
    assert_eq!(
        started.start_origin,
        PlayOrigin::Playlist,
        "the origin of the entry the command moved to"
    );
    let new_session = fresh[1].session;
    assert_ne!(new_session, rig.first_session_id());
    assert_eq!(fresh[2].session, new_session);
    assert_eq!(
        fresh[2].recording,
        Some(recording(1)),
        "the old play's outcome"
    );
    let SignalPayload::Skipped(p) = fresh[2].payload else {
        panic!("kind");
    };
    assert_eq!(p.input, InputSource::MediaKey);
    assert_eq!(rig.ctl.status().session, Some(new_session));

    // The new track was loaded paused; once it is heard it starts in the new session.
    rig.ctl.resume(InputSource::MediaKey);
    rig.hear();
    let heard = rig.new_signals();
    assert_eq!(
        kinds(&heard),
        [SignalKind::Resumed, SignalKind::PlayStarted]
    );
    assert_eq!(heard[1].session, new_session);
    assert_eq!(heard[1].recording, Some(recording(2)));
}

#[test]
fn next_after_29_59_of_pause_stays_in_the_same_session() {
    let (mut rig, fresh) = next_after_pause("ses-stale-skip-early", HALF_HOUR_MS - 1_000);
    assert_eq!(kinds(&fresh), [SignalKind::Skipped]);
    assert_eq!(fresh[0].session, rig.first_session_id());
    rig.ctl.resume(InputSource::MediaKey);
    rig.hear();
    let heard = rig.new_signals();
    assert_eq!(
        kinds(&heard),
        [SignalKind::Resumed, SignalKind::PlayStarted]
    );
    assert_eq!(heard[1].session, rig.first_session_id());
}

#[test]
fn a_chosen_track_after_a_long_pause_opens_the_session_with_the_chosen_origin() {
    let (mut rig, _) = paused_after_two_minutes("ses-stale-select");
    let b = rig.wav("b.wav", 10 * SECOND);
    let a = rig.dir.path("a.wav");
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 10 * SECOND, QueueOrigin::Manual),
    ];
    rig.clock.advance(HALF_HOUR_MS + MINUTE_MS);
    rig.load_with(
        entries,
        1,
        LoadIntent::Select(PlayOrigin::Search),
        InputSource::Mouse,
    )
    .expect("load");
    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [
            SignalKind::SessionEnded,
            SignalKind::SessionStarted,
            SignalKind::Skipped
        ]
    );
    let SignalPayload::SessionStarted(started) = fresh[1].payload else {
        panic!("kind");
    };
    assert_eq!(started.start_origin, PlayOrigin::Search);
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
    rig.hear();
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
