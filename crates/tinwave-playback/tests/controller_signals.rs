//! One emission test per mapped signal kind, through the real engine (on a `ManualOutput`), the
//! real `Controller` and a real `SignalBus`: exactly one event, with the right payload, position
//! and input source. Plus the outcome rules (failed, seek to the end, gestures across tracks) and
//! the privacy rule (a disabled category constructs nothing).
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use controller_support::*;
use tinwave_playback::{LoadIntent, ManualOutput, PlaybackError, StreamInfo, StreamMode};
use tinwave_signals::{
    Fraction, InputSource, PlayOrigin, PositionDs, QueueOrigin, Rate, RepeatMode, Section,
    SignalCategory, SignalKind, SignalPayload, Volume,
};

const SECOND: usize = 48_000;

fn ds(ms: u64) -> PositionDs {
    PositionDs::from_ms(ms)
}

#[test]
fn play_started_carries_origin_input_recording_source_and_start_position() {
    let mut rig = Rig::new("sig-play-started");
    let a = rig.wav("a.wav", SECOND);
    let e = rig.entry(1, &a, SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Keyboard);
    assert!(
        rig.signals().is_empty(),
        "nothing has been heard yet, so nothing has started"
    );
    rig.hear();

    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [SignalKind::SessionStarted, SignalKind::PlayStarted]
    );
    let started = only(&signals, SignalKind::PlayStarted);
    let SignalPayload::PlayStarted(p) = started.payload else {
        panic!("kind");
    };
    assert_eq!(
        (p.origin, p.input),
        (PlayOrigin::Album, InputSource::Keyboard)
    );
    assert_eq!(started.recording, Some(recording(1)));
    assert_eq!(started.source, Some(source(1)));
    assert_eq!(started.pos_ds, Some(0));
    assert_eq!(started.session, rig.first_session_id());
    assert_eq!(started.ts.as_i64(), T0);
    assert_eq!(started.ctx, None);
}

#[test]
fn an_auto_advance_is_auto_with_the_origin_of_the_queue_entry() {
    let mut rig = Rig::new("sig-advance");
    let a = rig.wav("a.wav", SECOND / 2);
    let b = rig.wav("b.wav", SECOND / 2);
    let entries = vec![
        entry(1, &a, SECOND / 2, QueueOrigin::Album),
        entry(2, &b, SECOND / 2, QueueOrigin::Radio),
    ];
    rig.select(entries, 0, PlayOrigin::Search, InputSource::Mouse);
    rig.play_to_end();

    let starts: Vec<_> = rig
        .of_kind(SignalKind::PlayStarted)
        .into_iter()
        .map(|e| match e.payload {
            SignalPayload::PlayStarted(p) => (e.recording, p.origin, p.input),
            _ => panic!("kind"),
        })
        .collect();
    assert_eq!(
        starts,
        [
            (Some(recording(1)), PlayOrigin::Search, InputSource::Mouse),
            (Some(recording(2)), PlayOrigin::Radio, InputSource::Auto),
        ]
    );
}

#[test]
fn completed_counts_listening_time_without_the_pauses() {
    let mut rig = Rig::new("sig-completed");
    let a = rig.wav("a.wav", 2 * SECOND);
    let e = rig.entry(1, &a, 2 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(4_800, 480);
    rig.clock.advance(1_000);
    rig.ctl.pause(InputSource::MediaKey);
    rig.clock.advance(5_000);
    rig.ctl.resume(InputSource::MediaKey);
    rig.clock.advance(2_000);
    rig.play_to_end();

    let signals = rig.signals();
    let completed = only(&signals, SignalKind::Completed);
    let SignalPayload::Completed(p) = completed.payload else {
        panic!("kind");
    };
    // 8 s of wall time since the track started, 5 s of it paused.
    assert_eq!(p.listened_ds, ds(3_000));
    assert_eq!(p.paused_ds, ds(5_000));
    assert_eq!(completed.recording, Some(recording(1)));
    assert_eq!(completed.pos_ds, Some(20), "left at its 2 s end");
    assert!(
        !kinds(&signals).contains(&SignalKind::Skipped),
        "{:?}",
        kinds(&signals)
    );
}

#[test]
fn skipped_has_position_fraction_input_and_queue_origin() {
    let mut rig = Rig::new("sig-skipped");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Playlist),
        entry(2, &b, 10 * SECOND, QueueOrigin::Radio),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Playlist, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.new_signals();
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::MediaKey)
        .expect("next");

    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::Skipped],
        "the skip now; the next play starts when it is heard"
    );
    let skipped = only(&fresh, SignalKind::Skipped);
    let SignalPayload::Skipped(p) = skipped.payload else {
        panic!("kind");
    };
    assert_eq!(p.input, InputSource::MediaKey);
    assert_eq!(p.queue_origin, QueueOrigin::Playlist);
    assert_eq!(p.section, Section::UNKNOWN);
    assert_eq!(p.fraction, Fraction::from_raw(26), "1 s of 10 s");
    assert_eq!(skipped.pos_ds, Some(10));
    assert_eq!(skipped.recording, Some(recording(1)));
    rig.hear();
    let fresh = rig.new_signals();
    let SignalPayload::PlayStarted(started) = only(&fresh, SignalKind::PlayStarted).payload else {
        panic!("kind");
    };
    assert_eq!(
        (started.origin, started.input),
        (PlayOrigin::Radio, InputSource::MediaKey),
        "next uses the entry's origin and the command's input"
    );
}

#[test]
fn leaving_a_track_in_its_last_three_percent_is_completed_not_skipped() {
    let mut rig = Rig::new("sig-near-end");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 10 * SECOND, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    // 97.5 % of the way.
    rig.render_exact(10 * SECOND * 975 / 1000, 4_800);
    rig.new_signals();
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::Keyboard)
        .expect("next");
    assert_eq!(kinds(&rig.new_signals()), [SignalKind::Completed]);
    rig.hear();
    assert_eq!(kinds(&rig.new_signals()), [SignalKind::PlayStarted]);
}

#[test]
fn previous_emits_previous_pressed_for_the_track_it_returns_to_and_skips_the_one_it_leaves() {
    let mut rig = Rig::new("sig-previous");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Album),
        entry(2, &b, 10 * SECOND, QueueOrigin::Album),
    ];
    rig.select(entries.clone(), 1, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.new_signals();
    rig.load_with(entries, 0, LoadIntent::Previous, InputSource::Keyboard)
        .expect("previous");

    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::Skipped, SignalKind::PreviousPressed],
        "the press is the listener's act, whether or not the track then plays"
    );
    let skipped = only(&fresh, SignalKind::Skipped);
    assert_eq!(skipped.recording, Some(recording(2)));
    let previous = only(&fresh, SignalKind::PreviousPressed);
    assert_eq!(
        previous.recording,
        Some(recording(1)),
        "the track returned to"
    );
    assert_eq!(previous.pos_ds, Some(0));
    rig.hear();
    assert_eq!(kinds(&rig.new_signals()), [SignalKind::PlayStarted]);
}

#[test]
fn a_failed_track_is_neither_completed_nor_skipped() {
    let mut rig = Rig::new("sig-failed");
    let a = rig.wav("a.wav", SECOND / 2);
    let missing = rig.dir.path("missing.wav");
    let c = rig.wav("c.wav", SECOND / 2);
    let entries = vec![
        entry(1, &a, SECOND / 2, QueueOrigin::Manual),
        entry(2, &missing, SECOND / 2, QueueOrigin::Manual),
        entry(3, &c, SECOND / 2, QueueOrigin::Manual),
    ];
    rig.select(entries, 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();

    let signals = rig.signals();
    let starts: Vec<_> = signals
        .iter()
        .filter(|e| e.kind() == SignalKind::PlayStarted)
        .map(|e| e.recording)
        .collect();
    assert_eq!(
        starts,
        [Some(recording(1)), Some(recording(3))],
        "the track that failed to open was never heard, so it never started"
    );
    let outcomes: Vec<_> = signals
        .iter()
        .filter(|e| matches!(e.kind(), SignalKind::Completed | SignalKind::Skipped))
        .map(|e| (e.kind(), e.recording))
        .collect();
    assert_eq!(
        outcomes,
        [
            (SignalKind::Completed, Some(recording(1))),
            (SignalKind::Completed, Some(recording(3))),
        ],
        "no outcome for the track that failed to open: {:?}",
        kinds(&signals)
    );
}

#[test]
fn a_queue_of_only_a_failing_track_leaves_no_outcome_either() {
    let mut rig = Rig::new("sig-failed-only");
    let missing = rig.dir.path("missing.wav");
    let e = entry(1, &missing, SECOND, QueueOrigin::Manual);
    rig.select(vec![e], 0, PlayOrigin::Search, InputSource::Mouse);
    rig.run_until("the failed track is closed", |r| {
        r.ctl.status().recording.is_none()
    });
    assert_eq!(
        kinds(&rig.signals()),
        [],
        "a track that fails before any audio is not a play, and is not a session"
    );
}

#[test]
fn stopping_before_the_last_three_percent_records_no_outcome() {
    let mut rig = Rig::new("sig-stopped");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.new_signals();
    rig.ctl.stop().expect("stop");
    assert!(rig.new_signals().is_empty());
    assert_eq!(rig.ctl.status().recording, None);
}

#[test]
fn pause_and_resume_carry_the_input_and_the_pause_length() {
    let mut rig = Rig::new("sig-pause");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.new_signals();

    rig.ctl.pause(InputSource::Tray);
    rig.clock.advance(12_340);
    rig.ctl.resume(InputSource::Keyboard);
    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::Paused, SignalKind::Resumed]);
    let SignalPayload::Paused(paused) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!(paused.input, InputSource::Tray);
    assert_eq!(fresh[0].pos_ds, Some(10));
    assert_eq!(fresh[0].recording, Some(recording(1)));
    let SignalPayload::Resumed(resumed) = fresh[1].payload else {
        panic!("kind");
    };
    assert_eq!(resumed.input, InputSource::Keyboard);
    assert_eq!(resumed.pause_ds, ds(12_340));
    assert_eq!(fresh[1].pos_ds, Some(10));
}

#[test]
fn pausing_twice_or_with_nothing_loaded_emits_nothing() {
    let mut rig = Rig::new("sig-pause-idle");
    rig.ctl.pause(InputSource::Mouse);
    rig.ctl.resume(InputSource::Mouse);
    assert!(rig.signals().is_empty(), "no track, no session, no signal");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();
    rig.ctl.pause(InputSource::Mouse);
    rig.ctl.pause(InputSource::Mouse);
    assert_eq!(kinds(&rig.new_signals()), [SignalKind::Paused]);
}

#[test]
fn a_seek_over_two_seconds_is_one_event_after_its_window() {
    let mut rig = Rig::new("sig-seek-forward");
    let a = rig.wav("a.wav", 20 * SECOND);
    let e = rig.entry(1, &a, 20 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.new_signals();

    rig.ctl.seek_ms(8_000, InputSource::Keyboard).expect("seek");
    rig.clock.advance(1_999);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty(), "the gesture is still open");
    rig.clock.advance(1);
    rig.ctl.poll();

    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::SeekForward]);
    let SignalPayload::SeekForward(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!((p.from_ds, p.to_ds), (ds(1_000), ds(8_000)));
    assert_eq!(p.input, InputSource::Keyboard);
    assert_eq!(
        (p.section_from, p.section_to),
        (Section::UNKNOWN, Section::UNKNOWN)
    );
    assert_eq!(fresh[0].pos_ds, Some(80));
    assert_eq!(fresh[0].recording, Some(recording(1)));
}

#[test]
fn a_seek_backwards_is_seek_back_and_small_seeks_are_nothing() {
    let mut rig = Rig::new("sig-seek-back");
    let a = rig.wav("a.wav", 20 * SECOND);
    let e = rig.entry(1, &a, 20 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(10 * SECOND, 4_800);
    rig.new_signals();

    rig.ctl
        .seek_ms(4_000, InputSource::Mouse)
        .expect("seek back");
    rig.clock.advance(2_000);
    rig.ctl.poll();
    // 1.5 s forward from there: below the 2 s threshold.
    rig.settle();
    rig.ctl.seek_ms(5_500, InputSource::Mouse).expect("nudge");
    rig.clock.advance(2_000);
    rig.ctl.poll();

    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::SeekBack]);
    let SignalPayload::SeekBack(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!((p.from_ds, p.to_ds), (ds(10_000), ds(4_000)));
}

#[test]
fn seeks_within_two_seconds_of_each_other_are_one_gesture() {
    let mut rig = Rig::new("sig-seek-merge");
    let a = rig.wav("a.wav", 30 * SECOND);
    let e = rig.entry(1, &a, 30 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();

    for target in [3_000, 6_000, 9_000] {
        rig.ctl.seek_ms(target, InputSource::Mouse).expect("seek");
        rig.clock.advance(500);
    }
    rig.clock.advance(2_000);
    rig.ctl.poll();
    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::SeekForward]);
    let SignalPayload::SeekForward(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!(
        (p.from_ds, p.to_ds),
        (ds(100), ds(9_000)),
        "first from, last to"
    );
}

#[test]
fn seeking_to_the_very_end_is_a_seek_and_a_completion() {
    let mut rig = Rig::new("sig-seek-end");
    let frames = 10 * SECOND;
    let a = rig.wav("a.wav", frames);
    let e = rig.entry(1, &a, frames);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.new_signals();

    rig.ctl
        .seek_ms(10_000, InputSource::Mouse)
        .expect("seek to the end");
    rig.settle();
    rig.run_until("the queue ended", |r| r.ctl.status().recording.is_none());

    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::SeekForward, SignalKind::Completed],
        "the gesture is flushed before the outcome"
    );
    let SignalPayload::SeekForward(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!((p.from_ds, p.to_ds), (ds(1_000), ds(10_000)));
}

#[test]
fn a_seek_gesture_never_merges_across_a_track_change() {
    let mut rig = Rig::new("sig-seek-track-change");
    let a = rig.wav("a.wav", 30 * SECOND);
    let e = rig.entry(1, &a, 30 * SECOND);
    rig.select(vec![e.clone()], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();

    rig.ctl.seek_ms(10_000, InputSource::Mouse).expect("seek");
    rig.clock.advance(100);
    // The same track loaded again: same recording, within the 2 s window.
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.settle();
    rig.hear();
    rig.clock.advance(100);
    rig.ctl
        .seek_ms(10_000, InputSource::Mouse)
        .expect("seek again");
    rig.clock.advance(2_000);
    rig.ctl.poll();

    let seeks: Vec<_> = rig
        .signals()
        .into_iter()
        .filter(|e| e.kind() == SignalKind::SeekForward)
        .collect();
    assert_eq!(seeks.len(), 2, "one per play, not one merged gesture");
    for seek in seeks {
        let SignalPayload::SeekForward(p) = seek.payload else {
            panic!("kind");
        };
        assert_eq!(p.to_ds, ds(10_000));
        assert!(
            p.from_ds.ms() < 1_000,
            "each starts near the start of its own play"
        );
    }
}

#[test]
fn a_seek_with_nothing_loaded_is_rejected_and_silent() {
    let mut rig = Rig::new("sig-seek-nothing");
    assert_eq!(
        rig.ctl.seek_ms(1_000, InputSource::Mouse),
        Err(PlaybackError::NothingLoaded)
    );
    assert!(rig.signals().is_empty());
}

#[test]
fn a_volume_gesture_is_one_event_with_first_from_last_to_and_the_input() {
    let mut rig = Rig::new("sig-volume");
    let a = rig.wav("a.wav", 30 * SECOND);
    let e = rig.entry(1, &a, 30 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(5 * SECOND, 4_800);
    rig.new_signals();

    for step in 1..=10_u8 {
        rig.ctl
            .set_volume(1.0 - 0.05 * f32::from(step), InputSource::Keyboard)
            .expect("volume");
        rig.clock.advance(50);
    }
    rig.clock.advance(549);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty(), "599 ms after the last tick");
    rig.clock.advance(1);
    rig.ctl.poll();

    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::VolumeChanged]);
    let SignalPayload::VolumeChanged(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!(p.from, Volume::MAX);
    assert_eq!(p.to, Volume::from_linear(0.5).expect("volume"));
    assert_eq!(p.input, InputSource::Keyboard);
    assert_eq!(p.section, Section::UNKNOWN);
    assert_eq!(p.since_track_start.ms(), 5_000);
    assert_eq!(fresh[0].recording, Some(recording(1)));
    assert_eq!(fresh[0].pos_ds, Some(50));
}

#[test]
fn a_rate_gesture_is_one_event_with_first_from_last_to_and_the_input() {
    let mut rig = Rig::new("sig-rate");
    let a = rig.wav("a.wav", 30 * SECOND);
    let e = rig.entry(1, &a, 30 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(5 * SECOND, 4_800);
    rig.new_signals();

    // 1.05, 1.10 ... 1.50: ten steps of a slider.
    for step in 1..=10_u8 {
        rig.ctl
            .set_rate(1.0 + 0.05 * f64::from(step), InputSource::Keyboard)
            .expect("rate");
        rig.clock.advance(50);
    }
    rig.clock.advance(549);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty(), "599 ms after the last tick");
    rig.clock.advance(1);
    rig.ctl.poll();

    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::RateChanged]);
    let SignalPayload::RateChanged(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!(p.from, Rate::NORMAL);
    assert_eq!(p.to, Rate::from_rate(1.5).expect("rate"));
    assert_eq!(p.input, InputSource::Keyboard);
    assert_eq!(p.section, Section::UNKNOWN);
    assert_eq!(fresh[0].recording, Some(recording(1)));
    assert_eq!(fresh[0].pos_ds, Some(50));
    assert!((rig.ctl.rate() - 1.5).abs() < f64::EPSILON);
}

#[test]
fn a_rate_gesture_that_returns_to_where_it_started_is_not_a_change() {
    let mut rig = Rig::new("sig-rate-back");
    let a = rig.wav("a.wav", 30 * SECOND);
    let e = rig.entry(1, &a, 30 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 4_800);
    rig.new_signals();
    rig.ctl.set_rate(1.25, InputSource::Mouse).expect("rate");
    rig.clock.advance(100);
    rig.ctl.set_rate(1.0, InputSource::Mouse).expect("rate");
    rig.clock.advance(700);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty());
}

#[test]
fn a_rate_gesture_ends_at_a_track_change() {
    let mut rig = Rig::new("sig-rate-track-change");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 10 * SECOND, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();
    rig.ctl.set_rate(0.8, InputSource::Mouse).expect("rate");
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::Mouse)
        .expect("next");

    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::RateChanged, SignalKind::Skipped]
    );
    assert_eq!(fresh[0].recording, Some(recording(1)));
}

#[test]
fn a_rate_change_with_no_session_is_applied_but_not_recorded() {
    let mut rig = Rig::new("sig-rate-no-session");
    rig.ctl.set_rate(1.25, InputSource::Mouse).expect("rate");
    rig.clock.advance(1_000);
    rig.ctl.poll();
    assert!(rig.signals().is_empty());
    assert!((rig.ctl.rate() - 1.25).abs() < f64::EPSILON);
}

#[test]
fn a_rate_outside_the_range_is_refused_and_silent() {
    let mut rig = Rig::new("sig-rate-range");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();
    for bad in [0.5, 2.0, f64::NAN] {
        assert_eq!(
            rig.ctl.set_rate(bad, InputSource::Mouse),
            Err(PlaybackError::InvalidArgument(
                "rate must be within 0.75 to 1.5"
            ))
        );
    }
    rig.clock.advance(1_000);
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty());
    assert!((rig.ctl.rate() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn a_rate_change_on_a_play_not_yet_heard_is_recorded() {
    let mut rig = Rig::new("sig-rate-unheard");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Search, InputSource::Mouse);
    rig.ctl.set_rate(0.75, InputSource::Keyboard).expect("rate");
    rig.clock.advance(1_000);
    rig.ctl.poll();
    assert_eq!(
        kinds(&rig.signals()),
        [SignalKind::SessionStarted, SignalKind::RateChanged]
    );
}

#[test]
fn a_volume_gesture_ends_at_a_track_change() {
    let mut rig = Rig::new("sig-volume-track-change");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 10 * SECOND, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();
    rig.ctl.set_volume(0.5, InputSource::Mouse).expect("volume");
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::Mouse)
        .expect("next");

    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::VolumeChanged, SignalKind::Skipped]
    );
    assert_eq!(fresh[0].recording, Some(recording(1)));
}

#[test]
fn volume_with_no_session_is_applied_but_not_recorded() {
    let mut rig = Rig::new("sig-volume-no-session");
    rig.ctl.set_volume(0.3, InputSource::Mouse).expect("volume");
    rig.clock.advance(1_000);
    rig.ctl.poll();
    assert!(rig.signals().is_empty());
    assert!((rig.ctl.volume() - 0.3).abs() < f32::EPSILON);
}

#[test]
fn repeat_mode_changed_is_emitted_once_per_change() {
    let mut rig = Rig::new("sig-repeat");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();
    rig.ctl.set_repeat_mode(RepeatMode::One);
    rig.ctl.set_repeat_mode(RepeatMode::One);
    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::RepeatModeChanged]);
    let SignalPayload::RepeatModeChanged(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!(p.mode, RepeatMode::One);
    assert_eq!(fresh[0].recording, Some(recording(1)));
    assert_eq!(rig.ctl.status().repeat, RepeatMode::One);
}

#[test]
fn the_mode_of_a_reopened_stream_alone_never_emits_exclusive_mode_toggled() {
    // The listener's setting never changes (this output reports "not exclusive"), whatever the
    // streams say: exclusive, exclusive again, then a fallback to shared.
    let mut exclusive = StreamInfo::unknown();
    exclusive.mode = StreamMode::Exclusive;
    let mut shared_fallback = StreamInfo::unknown();
    shared_fallback.fallback = Some("device refused exclusive mode".into());
    let (output, handle, _infos) = InfoOutput::new(vec![
        StreamInfo::unknown(),
        exclusive.clone(),
        exclusive,
        shared_fallback,
    ]);
    let mut rig = Rig::with_output("sig-exclusive", Box::new(output), handle);
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    rig.new_signals();

    for device in [Some("dac"), Some("dac"), None] {
        rig.ctl
            .switch_device(device.map(str::to_owned))
            .expect("switch");
        assert!(
            rig.new_signals().is_empty(),
            "a stream that came back {device:?} is no preference"
        );
    }
}

#[test]
fn disabling_a_category_constructs_nothing_of_it() {
    let mut rig = Rig::new("sig-disabled");
    let settings = rig.bus.settings().clone();
    settings.set_category(SignalCategory::SeekSkipReplay, false);
    settings.set_category(SignalCategory::Volume, false);
    settings.set_category(SignalCategory::Speed, false);

    let a = rig.wav("a.wav", 20 * SECOND);
    let b = rig.wav("b.wav", 20 * SECOND);
    let entries = vec![
        entry(1, &a, 20 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 20 * SECOND, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.ctl.pause(InputSource::Mouse);
    rig.ctl.resume(InputSource::Mouse);
    rig.ctl.seek_ms(12_000, InputSource::Mouse).expect("seek");
    rig.ctl.set_volume(0.4, InputSource::Mouse).expect("volume");
    rig.ctl.set_rate(1.25, InputSource::Mouse).expect("rate");
    rig.clock.advance(3_000);
    rig.ctl.poll();
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::Mouse)
        .expect("next");
    rig.hear();
    rig.ctl.set_repeat_mode(RepeatMode::All);

    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::PlayStarted
        ],
        "only the enabled categories (session, queue/library) produced events"
    );
    // A kind that is off consumes no sequence number: nothing was constructed for it.
    let seqs: Vec<_> = signals.iter().map(|e| e.id.seq).collect();
    assert_eq!(seqs, [0, 1, 2]);
    assert_eq!(rig.bus.dropped_total(), 0);
}

#[test]
fn a_zero_duration_is_rejected_up_front() {
    let mut rig = Rig::new("sig-zero-duration");
    let a = rig.wav("a.wav", SECOND);
    let mut e = rig.entry(1, &a, SECOND);
    e.duration_ms = 0;
    assert_eq!(
        rig.load_with(
            vec![e],
            0,
            LoadIntent::Select(PlayOrigin::Album),
            InputSource::Mouse
        ),
        Err(PlaybackError::InvalidArgument(
            "duration_ms must be above 0"
        ))
    );
    assert!(rig.signals().is_empty());
}

#[test]
fn with_every_category_off_nothing_is_constructed_and_playback_goes_on() {
    let mut rig = Rig::new("sig-all-off");
    let settings = rig.bus.settings().clone();
    for category in SignalCategory::ALL {
        settings.set_category(category, false);
    }
    let a = rig.wav("a.wav", 20 * SECOND);
    let e = rig.entry(1, &a, 20 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(SECOND, 480);
    rig.ctl.pause(InputSource::Mouse);
    rig.ctl.resume(InputSource::Mouse);
    rig.ctl.seek_ms(12_000, InputSource::Mouse).expect("seek");
    rig.ctl.set_volume(0.4, InputSource::Mouse).expect("volume");
    rig.clock.advance(3_000);
    rig.ctl.poll();
    rig.ctl.stop().expect("stop");
    assert!(rig.signals().is_empty());
    assert_eq!(rig.bus.queued_total(), 0);
    assert_eq!(rig.bus.dropped_total(), 0);

    // Switching a category back on takes effect at the next signal, and no sequence number was
    // spent while it was off.
    settings.set_category(SignalCategory::SeekSkipReplay, true);
    let b = rig.wav("b.wav", 10 * SECOND);
    let e = rig.entry(2, &b, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_exact(480, 480);
    rig.ctl.pause(InputSource::Mouse);
    let signals = rig.signals();
    assert_eq!(kinds(&signals), [SignalKind::Paused]);
    assert_eq!(signals[0].id.seq, 0);
}

#[test]
fn the_engine_reports_how_each_stream_was_opened_the_first_one_too() {
    use std::sync::Arc;
    use tinwave_core::{ManualClock, UtcMillis};
    use tinwave_playback::{Engine, EngineConfig, EventKind, FsOpener};

    let mut opened = StreamInfo::unknown();
    opened.backend = "wasapi";
    opened.mode = StreamMode::Shared;
    opened.period_frames = 480;
    opened.stream_latency_frames = 96;
    opened.fallback = Some("exclusive mode refused: format not supported".into());
    let mut initial = StreamInfo::unknown();
    initial.backend = "wasapi";
    initial.mode = StreamMode::Exclusive;
    initial.period_frames = 240;
    let (output, _handle, _) = InfoOutput::new(vec![initial.clone(), opened.clone()]);
    let sink = Arc::new(common::CollectingSink::default());
    let mut engine = Engine::start(
        EngineConfig {
            sample_rate: common::RATE,
            device: None,
            opener: Arc::new(FsOpener),
            sink: sink.clone(),
            clock: Arc::new(ManualClock::new(UtcMillis::new(T0))),
        },
        Box::new(output),
    )
    .expect("engine");
    assert_eq!(
        sink.take(),
        vec![EventKind::OutputOpened {
            device: None,
            info: initial
        }],
        "the starting stream is observable without a device switch"
    );
    engine.switch_device(Some("dac".into())).expect("switch");
    assert_eq!(
        sink.take(),
        vec![EventKind::OutputReopened {
            device: Some("dac".into()),
            info: opened
        }]
    );
    engine.shutdown();
}

#[test]
fn a_repeat_mode_set_with_nothing_loaded_is_reported_by_the_next_session() {
    let mut rig = Rig::new("sig-repeat-no-session");
    rig.ctl.set_repeat_mode(RepeatMode::One);
    assert!(rig.signals().is_empty(), "no session yet");
    assert_eq!(rig.ctl.status().repeat, RepeatMode::One);

    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    assert!(rig.signals().is_empty(), "nothing is heard yet");
    rig.hear();
    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::RepeatModeChanged,
            SignalKind::PlayStarted
        ]
    );
    let SignalPayload::RepeatModeChanged(p) = signals[1].payload else {
        panic!("kind");
    };
    assert_eq!(p.mode, RepeatMode::One);
    assert_eq!(signals[1].session, signals[0].session);
}

#[test]
fn a_repeat_mode_changed_back_before_any_session_is_not_reported() {
    let mut rig = Rig::new("sig-repeat-undone");
    rig.ctl.set_repeat_mode(RepeatMode::One);
    rig.ctl.set_repeat_mode(RepeatMode::Off);
    let a = rig.wav("a.wav", SECOND);
    let e = rig.entry(1, &a, SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    assert_eq!(
        kinds(&rig.signals()),
        [SignalKind::SessionStarted, SignalKind::PlayStarted]
    );
}

#[test]
fn a_repeat_mode_set_after_a_session_ended_is_reported_by_the_next_one() {
    let mut rig = Rig::new("sig-repeat-after-end");
    let a = rig.wav("a.wav", SECOND / 2);
    let e = rig.entry(1, &a, SECOND / 2);
    rig.select(vec![e.clone()], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();
    rig.clock.advance(30 * 60_000);
    rig.ctl.poll();
    rig.new_signals();

    rig.ctl.set_repeat_mode(RepeatMode::All);
    assert!(rig.new_signals().is_empty());
    rig.select(vec![e], 0, PlayOrigin::Search, InputSource::Mouse);
    rig.hear();
    assert_eq!(
        kinds(&rig.new_signals()),
        [
            SignalKind::SessionStarted,
            SignalKind::RepeatModeChanged,
            SignalKind::PlayStarted
        ]
    );
}

#[test]
fn an_exclusive_setting_change_outside_a_session_is_reported_by_the_next_session() {
    let mut rig = Rig::new("sig-exclusive-no-session");
    rig.out.set_exclusive(true);
    rig.ctl.poll();
    assert!(rig.signals().is_empty(), "no session yet");

    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::ExclusiveModeToggled,
            SignalKind::PlayStarted
        ]
    );
    let SignalPayload::ExclusiveModeToggled(p) = signals[1].payload else {
        panic!("kind");
    };
    assert!(p.on);
}

#[test]
fn an_exclusive_setting_changed_back_before_any_session_is_not_reported() {
    let mut rig = Rig::new("sig-exclusive-undone");
    rig.out.set_exclusive(true);
    rig.ctl.poll();
    rig.out.set_exclusive(false);
    rig.ctl.poll();
    let a = rig.wav("a.wav", SECOND);
    let e = rig.entry(1, &a, SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    assert_eq!(
        kinds(&rig.signals()),
        [SignalKind::SessionStarted, SignalKind::PlayStarted]
    );
}

#[test]
fn the_setting_the_controller_started_with_is_the_starting_state_not_a_toggle() {
    let (output, handle) = ManualOutput::new();
    handle.set_exclusive(true);
    let mut rig = Rig::with_output("sig-exclusive-launch", Box::new(output), handle);
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    assert_eq!(
        kinds(&rig.new_signals()),
        [SignalKind::SessionStarted, SignalKind::PlayStarted],
        "starting in exclusive mode is not a change"
    );

    rig.ctl.switch_device(Some("dac".into())).expect("switch");
    rig.ctl.poll();
    assert!(rig.new_signals().is_empty(), "the setting did not move");
    rig.out.set_exclusive(false);
    rig.ctl.poll();
    let SignalPayload::ExclusiveModeToggled(p) =
        only(&rig.new_signals(), SignalKind::ExclusiveModeToggled).payload
    else {
        panic!("kind");
    };
    assert!(
        !p.on,
        "compared with the setting the controller started with"
    );
}

#[test]
fn a_restored_repeat_mode_is_the_starting_state_not_a_change() {
    let mut rig = Rig::with_repeat("sig-initial-repeat", RepeatMode::All);
    assert_eq!(rig.ctl.status().repeat, RepeatMode::All);
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    assert_eq!(
        kinds(&rig.new_signals()),
        [SignalKind::SessionStarted, SignalKind::PlayStarted],
        "restoring the saved mode at launch emits no RepeatModeChanged"
    );

    rig.ctl.set_repeat_mode(RepeatMode::All);
    assert!(rig.new_signals().is_empty(), "unchanged");
    rig.ctl.set_repeat_mode(RepeatMode::One);
    let fresh = rig.new_signals();
    assert_eq!(kinds(&fresh), [SignalKind::RepeatModeChanged]);
    let SignalPayload::RepeatModeChanged(p) = fresh[0].payload else {
        panic!("kind");
    };
    assert_eq!(p.mode, RepeatMode::One);
}

#[test]
fn a_track_replaced_before_its_first_audio_is_not_a_play() {
    let mut rig = Rig::new("sig-never-heard");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 10 * SECOND, QueueOrigin::Playlist),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::Mouse)
        .expect("next");
    assert!(
        rig.signals().is_empty(),
        "no start, no skip and no session for a track nobody heard: {:?}",
        kinds(&rig.signals())
    );

    rig.hear();
    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [SignalKind::SessionStarted, SignalKind::PlayStarted]
    );
    assert_eq!(signals[1].recording, Some(recording(2)));
    let SignalPayload::PlayStarted(p) = signals[1].payload else {
        panic!("kind");
    };
    assert_eq!(p.origin, PlayOrigin::Playlist);
}

#[test]
fn a_track_that_ends_before_its_first_position_mark_was_still_heard() {
    let mut rig = Rig::new("sig-short");
    let frames = SECOND / 20;
    let a = rig.wav("a.wav", frames);
    let e = rig.entry(1, &a, frames);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();
    assert_eq!(
        kinds(&rig.signals()),
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::Completed
        ],
        "50 ms is less than one mark, but it played to its end"
    );
}

#[test]
fn a_play_paused_before_it_is_heard_starts_a_session_with_the_pause() {
    let mut rig = Rig::new("sig-paused-unheard");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    assert!(rig.signals().is_empty(), "nothing yet");
    rig.ctl.pause(InputSource::Keyboard);
    rig.clock.advance(5_000);
    rig.ctl.resume(InputSource::MediaKey);
    rig.hear();
    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::Paused,
            SignalKind::Resumed,
            SignalKind::PlayStarted
        ],
        "the listener's pause and resume are recorded, in the session they began"
    );
    let SignalPayload::SessionStarted(p) = signals[0].payload else {
        panic!("kind");
    };
    assert_eq!(
        p.start_origin,
        PlayOrigin::Album,
        "the origin of the play, not of the resume"
    );
    let SignalPayload::Paused(paused) = signals[1].payload else {
        panic!("kind");
    };
    assert_eq!(paused.input, InputSource::Keyboard);
    let SignalPayload::Resumed(resumed) = signals[2].payload else {
        panic!("kind");
    };
    assert_eq!(
        (resumed.input, resumed.pause_ds),
        (InputSource::MediaKey, ds(5_000))
    );
}

#[test]
fn a_track_restored_paused_then_sought_and_resumed_records_what_the_listener_did() {
    let mut rig = Rig::new("sig-restored-paused");
    let a = rig.wav("a.wav", 20 * SECOND);
    let e = rig.entry(1, &a, 20 * SECOND);
    // The shell restores the last track paused: its own commands are `Auto`, not the listener's.
    rig.ctl.pause(InputSource::Auto);
    rig.select(vec![e], 0, PlayOrigin::Resume, InputSource::Auto);
    assert!(
        rig.signals().is_empty(),
        "restoring is not the listener's act"
    );

    rig.ctl.seek_ms(5_000, InputSource::Mouse).expect("seek");
    rig.ctl.resume(InputSource::MediaKey);
    rig.clock.advance(2_000);
    rig.ctl.poll();
    rig.hear();

    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::Resumed,
            SignalKind::SeekForward,
            SignalKind::PlayStarted
        ]
    );
    let SignalPayload::SessionStarted(p) = signals[0].payload else {
        panic!("kind");
    };
    assert_eq!(p.start_origin, PlayOrigin::Resume);
    let SignalPayload::Resumed(resumed) = signals[1].payload else {
        panic!("kind");
    };
    assert_eq!(resumed.input, InputSource::MediaKey);
    let SignalPayload::SeekForward(seek) = signals[2].payload else {
        panic!("kind");
    };
    assert_eq!(
        (seek.from_ds, seek.to_ds, seek.input),
        (ds(0), ds(5_000), InputSource::Mouse)
    );
}

#[test]
fn a_volume_change_on_a_play_not_yet_heard_is_recorded() {
    let mut rig = Rig::new("sig-volume-unheard");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Search, InputSource::Mouse);
    rig.ctl
        .set_volume(0.5, InputSource::Keyboard)
        .expect("volume");
    rig.clock.advance(1_000);
    rig.ctl.poll();
    assert_eq!(
        kinds(&rig.signals()),
        [SignalKind::SessionStarted, SignalKind::VolumeChanged]
    );
}

#[test]
fn a_file_that_opens_but_has_no_audio_is_not_a_play() {
    let mut rig = Rig::new("sig-empty-file");
    let empty = rig.wav("empty.wav", 0);
    let a = rig.wav("a.wav", SECOND);
    let entries = vec![
        entry(1, &empty, SECOND, QueueOrigin::Manual),
        entry(2, &a, SECOND, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.run_until("the empty track was left", |r| {
        r.ctl.status().recording == Some(recording(2))
    });
    assert!(
        rig.signals().is_empty(),
        "no session, play or outcome for a track that decoded nothing: {:?}",
        kinds(&rig.signals())
    );
    rig.play_to_end();
    assert_eq!(
        kinds(&rig.signals()),
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::Completed
        ],
        "only the track that played"
    );
}

#[test]
fn a_seek_to_the_very_end_before_any_audio_is_not_a_play() {
    let mut rig = Rig::new("sig-seek-end-unheard");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.ctl.pause(InputSource::Auto);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Auto);
    rig.ctl.seek_ms(10_000, InputSource::Auto).expect("seek");
    rig.ctl.resume(InputSource::Auto);
    rig.settle();
    rig.run_until("the queue ended", |r| r.ctl.status().recording.is_none());
    assert!(rig.signals().is_empty(), "{:?}", kinds(&rig.signals()));
}

#[test]
fn previous_to_a_track_that_fails_still_records_the_press_and_its_session() {
    let mut rig = Rig::new("sig-previous-fails");
    let missing = rig.dir.path("missing.wav");
    let entries = vec![entry(1, &missing, SECOND, QueueOrigin::Album)];
    rig.load_with(entries, 0, LoadIntent::Previous, InputSource::Keyboard)
        .expect("previous");
    rig.run_until("the failed track is closed", |r| {
        r.ctl.status().recording.is_none()
    });
    let signals = rig.shutdown();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::PreviousPressed,
            SignalKind::SessionEnded
        ],
        "the listener's act is recorded; the track was never a play"
    );
    let SignalPayload::SessionEnded(p) = signals[2].payload else {
        panic!("kind");
    };
    assert_eq!(p.tracks, 0);
}

#[test]
fn play_started_reports_where_the_play_began() {
    let mut rig = Rig::new("sig-resume-position");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.ctl
        .load(tinwave_playback::LoadRequest {
            entries: vec![e],
            index: 0,
            start_frame: 2 * SECOND as u64,
            intent: LoadIntent::Select(PlayOrigin::Resume),
            input: InputSource::Auto,
            leaving: None,
        })
        .expect("load");
    rig.hear();
    let started = only(&rig.signals(), SignalKind::PlayStarted);
    assert_eq!(started.pos_ds, Some(20), "2 s in, not 2.1 s");
}

// --- RateSession and the applied rate (T-0062) ---------------------------------------------------

/// Sets the rate and acknowledges the stream restart with empty callbacks, which consume no audio,
/// until the engine reports the rate as applied: the frames played before and after are exactly
/// the ones the test renders. Waits on the condition (the decoder throttles restarts to one per
/// 125 ms), not on a sleep.
fn apply_rate(rig: &mut Rig, rate: f64) {
    rig.ctl
        .set_rate(rate, InputSource::Keyboard)
        .expect("set_rate");
    wait_applied(rig, rate);
}

fn wait_applied(rig: &mut Rig, rate: f64) {
    for _ in 0..30_000 {
        rig.out.render(&mut [], 0).expect("stream open");
        rig.ctl.poll();
        if rig.ctl.status().rate.to_bits() == rate.to_bits() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    panic!(
        "the engine never applied rate {rate}; status {:?}",
        rig.ctl.status()
    );
}

/// Within 0.1 % of `expected`.
fn near(measured: f64, expected: f64) -> bool {
    (measured - expected).abs() / expected < 0.001
}

#[test]
fn a_heard_play_has_one_rate_session_with_the_time_weighted_rate_of_its_schedule() {
    let mut rig = Rig::new("sig-ratesession");
    let a = rig.wav("a.wav", 30 * SECOND);
    let e = rig.entry(1, &a, 30 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    // 100 ms at 1.0 already played; then 1.25, 0.75 and 1.5 for 2 s each and 1 s back at 1.0.
    let mut weighted = 4_800.0;
    let mut played = 4_800.0;
    for (rate, frames) in [
        (1.25, 2 * SECOND),
        (0.75, 2 * SECOND),
        (1.5, 2 * SECOND),
        (1.0, SECOND),
    ] {
        apply_rate(&mut rig, rate);
        rig.render_exact(frames, 480);
        weighted += rate * frames as f64;
        played += frames as f64;
    }
    assert!(
        rig.ctl.take_rate_sessions().is_empty(),
        "a play that is still going has no value yet"
    );
    rig.clock.advance(5_000);
    rig.ctl.stop().expect("stop");

    let sessions = rig.ctl.take_rate_sessions();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let expected = weighted / played;
    assert!(
        near(sessions[0].time_weighted_rate, expected),
        "measured {}, the schedule is {expected}",
        sessions[0].time_weighted_rate
    );
    assert_eq!(sessions[0].recording, recording(1));
    assert_eq!(sessions[0].source, source(1));
    assert_eq!(sessions[0].at.as_i64(), T0 + 5_000, "when the play ended");
    assert!(
        rig.ctl.take_rate_sessions().is_empty(),
        "taken once, not again"
    );
    assert!(
        !kinds(&rig.signals()).contains(&SignalKind::RateSession),
        "a derived kind is never a bus event"
    );
}

#[test]
fn each_play_is_measured_on_its_own() {
    let mut rig = Rig::new("sig-ratesession-two");
    let a = rig.wav("a.wav", 10 * SECOND);
    let b = rig.wav("b.wav", 10 * SECOND);
    let entries = vec![
        entry(1, &a, 10 * SECOND, QueueOrigin::Manual),
        entry(2, &b, 10 * SECOND, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    apply_rate(&mut rig, 1.5);
    rig.render_exact(SECOND, 480);
    rig.load_with(entries, 1, LoadIntent::Next, InputSource::Mouse)
        .expect("next");
    // The new play runs at 1.0 only: the rate is put back before any of its audio is played.
    apply_rate(&mut rig, 1.0);
    rig.hear();
    rig.render_exact(SECOND, 480);
    rig.ctl.stop().expect("stop");

    let sessions = rig.ctl.take_rate_sessions();
    assert_eq!(
        sessions.iter().map(|s| s.recording).collect::<Vec<_>>(),
        [recording(1), recording(2)]
    );
    let first = (4_800.0 + 1.5 * SECOND as f64) / (4_800.0 + SECOND as f64);
    assert!(
        near(sessions[0].time_weighted_rate, first),
        "{} against {first}",
        sessions[0].time_weighted_rate
    );
    assert!(
        near(sessions[1].time_weighted_rate, 1.0),
        "the first play's 1.5 must not leak into the second: {}",
        sessions[1].time_weighted_rate
    );
}

#[test]
fn a_play_at_normal_speed_throughout_has_a_rate_session_of_one() {
    let mut rig = Rig::new("sig-ratesession-normal");
    let a = rig.wav("a.wav", SECOND);
    let e = rig.entry(1, &a, SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();
    let sessions = rig.ctl.take_rate_sessions();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert!(
        near(sessions[0].time_weighted_rate, 1.0),
        "{}",
        sessions[0].time_weighted_rate
    );
}

#[test]
fn no_rate_session_with_the_speed_category_off() {
    let mut rig = Rig::new("sig-ratesession-off");
    rig.bus
        .settings()
        .set_category(SignalCategory::Speed, false);
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();
    apply_rate(&mut rig, 1.25);
    rig.render_exact(SECOND, 480);
    rig.ctl.stop().expect("stop");
    assert_eq!(rig.ctl.take_rate_sessions(), []);

    // Switched back on, the next play is measured again.
    rig.bus.settings().set_category(SignalCategory::Speed, true);
    let b = rig.wav("b.wav", SECOND);
    let e = rig.entry(2, &b, SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();
    assert_eq!(rig.ctl.take_rate_sessions().len(), 1);
}

#[test]
fn no_rate_session_for_a_play_that_was_never_heard_or_could_not_be_played() {
    let mut rig = Rig::new("sig-ratesession-unheard");
    let a = rig.wav("a.wav", 10 * SECOND);
    let e = rig.entry(1, &a, 10 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    // Left before its first audio: not a play.
    rig.ctl.stop().expect("stop");
    assert_eq!(rig.ctl.take_rate_sessions(), []);

    // A track that fails to open is between two that play: only those have a value.
    let ok1 = rig.wav("ok1.wav", SECOND / 2);
    let missing = rig.dir.path("missing.wav");
    let ok2 = rig.wav("ok2.wav", SECOND / 2);
    let entries = vec![
        entry(1, &ok1, SECOND / 2, QueueOrigin::Manual),
        entry(2, &missing, SECOND / 2, QueueOrigin::Manual),
        entry(3, &ok2, SECOND / 2, QueueOrigin::Manual),
    ];
    rig.select(entries, 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();
    let sessions = rig.ctl.take_rate_sessions();
    assert_eq!(
        sessions.iter().map(|s| s.recording).collect::<Vec<_>>(),
        [recording(1), recording(3)],
        "{sessions:?}"
    );
}

#[test]
fn the_status_reports_the_rate_the_engine_applies_and_the_latest_change_wins() {
    let mut rig = Rig::new("sig-status-rate");
    assert_eq!(
        rig.ctl.status().rate.to_bits(),
        1.0f64.to_bits(),
        "nothing changed yet"
    );
    let a = rig.wav("a.wav", 30 * SECOND);
    let e = rig.entry(1, &a, 30 * SECOND);
    rig.select(vec![e], 0, PlayOrigin::Album, InputSource::Mouse);
    rig.hear();

    // A slider drag: the engine restarts its stream at most once per 125 ms, so the intermediate
    // values are not all applied.
    for rate in [1.25, 0.8, 1.5] {
        rig.ctl
            .set_rate(rate, InputSource::Mouse)
            .expect("set_rate");
    }
    assert!((rig.ctl.rate() - 1.5).abs() < f64::EPSILON, "the target");
    assert_eq!(
        rig.ctl.status().rate.to_bits(),
        1.0f64.to_bits(),
        "the audio is still at the old rate until the stream restarts: {:?}",
        rig.ctl.status()
    );
    wait_applied(&mut rig, 1.5);
    assert!(
        rig.ctl.status().is_live(),
        "playback went on through the restarts"
    );
}

#[test]
fn rate_sessions_stay_right_across_exclusive_mode_reopens() {
    // Exclusive mode (T-0057): the output is reopened at each file's rate, and `wall_frames`
    // counts frames of whichever rate played them. The ratio does not care.
    let (output, handle) = ManualOutput::new();
    handle.set_exclusive(true);
    let mut rig = Rig::with_output("sig-ratesession-exclusive", Box::new(output), handle);
    let mut entries = Vec::new();
    for (id, rate) in [(1u64, 44_100u32), (2, 96_000), (3, 44_100)] {
        let path = rig.dir.path(&format!("t{id}.wav"));
        common::write_sweep_piece(&path, rate, 0, rate as usize / 2, 2);
        entries.push(entry(id, &path, SECOND / 2, QueueOrigin::Manual));
    }
    // Nothing is playing yet, so the rate is adopted without a restart.
    apply_rate(&mut rig, 1.25);
    rig.select(entries, 0, PlayOrigin::Album, InputSource::Mouse);
    rig.play_to_end();

    assert_eq!(
        rig.out.opens().iter().map(|o| o.1).collect::<Vec<_>>(),
        [48_000, 44_100, 96_000, 44_100],
        "the output was reopened for each file"
    );
    let sessions = rig.ctl.take_rate_sessions();
    assert_eq!(
        sessions.iter().map(|s| s.recording).collect::<Vec<_>>(),
        [recording(1), recording(2), recording(3)]
    );
    for s in &sessions {
        assert!(
            near(s.time_weighted_rate, 1.25),
            "{} is not 1.25",
            s.time_weighted_rate
        );
    }
}

#[test]
fn the_handle_hands_over_rate_sessions_from_the_controller_thread() {
    use std::sync::Arc;
    use std::time::Duration;
    use tinwave_core::{DeviceId, IdGenerator, ManualClock, SeededRng, TzOffsetMin, UtcMillis};
    use tinwave_playback::{
        ControllerConfig, ControllerHandle, FixedTimezone, FsOpener, LoadRequest,
    };
    use tinwave_signals::{BusConfig, SignalBus, SignalSettings};

    let clock = ManualClock::new(UtcMillis::new(T0));
    let device = DeviceId::from_parts(UtcMillis::new(T0), [4; 10]).expect("id");
    let (bus, _drain) = SignalBus::channel(
        BusConfig::new(device),
        clock.clone(),
        Arc::new(SignalSettings::new()),
    )
    .expect("bus");
    let (output, out) = ManualOutput::new();
    let handle = ControllerHandle::spawn(
        ControllerConfig {
            sample_rate: common::RATE,
            device: None,
            opener: Arc::new(FsOpener),
            clock: Arc::new(clock.clone()),
            bus,
            ids: IdGenerator::new(clock, SeededRng::new(7)),
            timezone: Arc::new(FixedTimezone(TzOffsetMin::UTC)),
            initial_repeat: RepeatMode::Off,
            poll_interval: Duration::from_millis(2),
        },
        Box::new(output),
    )
    .expect("spawn");
    let dir = common::TempDir::new("sig-ratesession-handle");
    let (l, r) = common::sweep(common::RATE, SECOND / 2);
    let path = dir.path("a.wav");
    common::write_wav(&path, common::RATE, 16, &l, &r);
    handle
        .load(LoadRequest {
            entries: vec![entry(1, &path, SECOND / 2, QueueOrigin::Manual)],
            index: 0,
            start_frame: 0,
            intent: LoadIntent::Select(PlayOrigin::Album),
            input: InputSource::Mouse,
            leaving: None,
        })
        .expect("load");

    let mut block = vec![0.0f32; 960];
    let mut sessions = Vec::new();
    for _ in 0..30_000 {
        let real = out.render(&mut block, 0).unwrap_or(0);
        sessions.extend(handle.take_rate_sessions());
        if !sessions.is_empty() {
            break;
        }
        if real < 480 {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0].recording, recording(1));
    assert!(near(sessions[0].time_weighted_rate, 1.0));
    handle.shutdown();
}
