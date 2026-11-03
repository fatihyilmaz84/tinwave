//! Queue edits through the controller: each kind emits its catalog signal once, in the open session
//! and only when the engine applied the edit; a refused edit emits nothing; "next" and "previous"
//! pressed on a track that has already ended are ignored (P-0112).
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use controller_support::*;
use tinwave_playback::{
    LoadIntent, LoadRequest, PlaybackError, QueueEdit, QueueEditRequest, QueueEntry,
};
use tinwave_signals::{
    InputSource, PlayOrigin, QueueOrigin, SignalCategory, SignalKind, SignalPayload, View,
};

const SECOND: usize = 48_000;

fn edit(
    rig: &mut Rig,
    head: u64,
    tail: Vec<QueueEntry>,
    edit: QueueEdit,
    input: InputSource,
) -> Result<(), PlaybackError> {
    rig.ctl.edit_queue(QueueEditRequest {
        head: item(head),
        tail,
        edit,
        input,
    })
}

fn play_next(items: &[u64], view: View) -> QueueEdit {
    QueueEdit::PlayNext {
        items: items.iter().copied().map(item).collect(),
        from_view: view,
    }
}

/// Polls (no audio) until the decoder thread has answered every queue edit sent so far.
fn settle_edits(rig: &mut Rig) {
    for _ in 0..30_000 {
        rig.ctl.poll();
        if rig.ctl.diagnostics().edits_pending == 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    panic!("the decoder never answered: {:?}", rig.ctl.diagnostics());
}

/// A rig playing `a` (id 1) with `b` (id 2) queued behind it, both 20 s, `a` heard.
fn playing(tag: &str) -> (Rig, Vec<QueueEntry>) {
    let mut rig = Rig::new(tag);
    let (a, b) = (rig.wav("a.wav", 20 * SECOND), rig.wav("b.wav", 20 * SECOND));
    let entries = vec![rig.entry(1, &a, 20 * SECOND), rig.entry(2, &b, 20 * SECOND)];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_until_heard();
    (rig, entries)
}

fn extra(rig: &Rig, id: u64, origin: QueueOrigin) -> QueueEntry {
    let path = rig.wav(&format!("extra{id}.wav"), 20 * SECOND);
    entry(id, &path, 20 * SECOND, origin)
}

#[test]
fn play_next_emits_one_queued_next_per_inserted_item_in_the_open_session() {
    let (mut rig, entries) = playing("edit-sig-next");
    let (x, y) = (
        extra(&rig, 3, QueueOrigin::Manual),
        extra(&rig, 4, QueueOrigin::Manual),
    );
    let tail = vec![x, y, entries[1].clone()];
    edit(
        &mut rig,
        1,
        tail,
        play_next(&[3, 4], View::Search),
        InputSource::Mouse,
    )
    .expect("edit");
    settle_edits(&mut rig);

    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::QueuedNext,
            SignalKind::QueuedNext
        ],
        "the playing track was not interrupted and each inserted item has its signal"
    );
    let session = signals[0].session;
    for (event, id) in signals[2..].iter().zip([3, 4]) {
        assert_eq!(event.session, session);
        assert_eq!(event.recording, Some(recording(id)));
        assert_eq!(event.source, Some(source(id)));
        assert_eq!(event.pos_ds, None);
        let SignalPayload::QueuedNext(p) = event.payload else {
            panic!("kind");
        };
        assert_eq!(p.from_view, View::Search);
    }
    assert_eq!(
        rig.ctl.position().map(|p| p.item),
        Some(item(1)),
        "still the same track"
    );
}

#[test]
fn queue_later_emits_one_queued_later_per_added_entry() {
    let (mut rig, _) = playing("edit-sig-later");
    let added: Vec<_> = (7..=9)
        .map(|id| extra(&rig, id, QueueOrigin::Album))
        .collect();
    rig.ctl
        .queue_later(added, View::Album, InputSource::Keyboard)
        .expect("queue later");

    let signals = rig.signals();
    assert_eq!(
        kinds(&signals)[2..],
        [SignalKind::QueuedLater; 3],
        "{:?}",
        kinds(&signals)
    );
    for (event, id) in signals[2..].iter().zip(7..=9) {
        assert_eq!(event.session, signals[0].session);
        assert_eq!(event.recording, Some(recording(id)));
        let SignalPayload::QueuedLater(p) = event.payload else {
            panic!("kind");
        };
        assert_eq!(p.from_view, View::Album);
    }
}

#[test]
fn reordering_emits_queue_reordered_with_the_signed_distance() {
    let (mut rig, entries) = playing("edit-sig-reorder");
    let c = extra(&rig, 3, QueueOrigin::Manual);
    let b = entries[1].clone();
    edit(
        &mut rig,
        1,
        vec![b.clone(), c.clone()],
        play_next(&[2], View::Library),
        InputSource::Mouse,
    )
    .expect("setup edit");
    settle_edits(&mut rig);
    let _ = rig.new_signals();

    // C dragged ahead of B, then back behind it.
    edit(
        &mut rig,
        1,
        vec![c.clone(), b.clone()],
        QueueEdit::Reorder {
            item: item(3),
            distance: -1,
        },
        InputSource::Mouse,
    )
    .expect("up");
    edit(
        &mut rig,
        1,
        vec![b, c],
        QueueEdit::Reorder {
            item: item(3),
            distance: 1,
        },
        InputSource::Mouse,
    )
    .expect("down");
    settle_edits(&mut rig);

    let fresh = rig.new_signals();
    assert_eq!(
        kinds(&fresh),
        [SignalKind::QueueReordered, SignalKind::QueueReordered]
    );
    let distances: Vec<_> = fresh
        .iter()
        .map(|e| {
            let SignalPayload::QueueReordered(p) = e.payload else {
                panic!("kind");
            };
            (e.recording, p.distance)
        })
        .collect();
    assert_eq!(
        distances,
        [(Some(recording(3)), -1), (Some(recording(3)), 1)]
    );
}

#[test]
fn removing_emits_removed_from_queue_and_knows_a_radio_pick() {
    let mut rig = Rig::new("edit-sig-remove");
    let a = rig.wav("a.wav", 20 * SECOND);
    let entries = vec![
        rig.entry(1, &a, 20 * SECOND),
        extra(&rig, 2, QueueOrigin::Radio),
        extra(&rig, 3, QueueOrigin::Manual),
        extra(&rig, 4, QueueOrigin::Manual),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_until_heard();

    // The radio pick and a manual entry in one call, then the last one on its own.
    edit(
        &mut rig,
        1,
        vec![entries[3].clone()],
        QueueEdit::Remove {
            items: vec![item(2), item(3)],
        },
        InputSource::Mouse,
    )
    .expect("remove two");
    edit(
        &mut rig,
        1,
        Vec::new(),
        QueueEdit::Remove {
            items: vec![item(4)],
        },
        InputSource::Keyboard,
    )
    .expect("remove the last");
    settle_edits(&mut rig);

    let removed: Vec<_> = rig
        .signals()
        .into_iter()
        .filter(|e| e.kind() == SignalKind::RemovedFromQueue)
        .map(|e| {
            let SignalPayload::RemovedFromQueue(p) = e.payload else {
                panic!("kind");
            };
            (e.recording, p.was_radio_pick)
        })
        .collect();
    assert_eq!(
        removed,
        [
            (Some(recording(2)), true),
            (Some(recording(3)), false),
            (Some(recording(4)), false)
        ]
    );
}

#[test]
fn an_edit_built_on_a_track_that_has_ended_emits_nothing() {
    let mut rig = Rig::new("edit-sig-stale");
    // A ends after half a second and B follows by itself.
    let (a, b) = (rig.wav("a.wav", SECOND / 2), rig.wav("b.wav", 20 * SECOND));
    let entries = vec![rig.entry(1, &a, SECOND / 2), rig.entry(2, &b, 20 * SECOND)];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_until_heard();
    rig.run_until("B is playing", |r| {
        r.ctl.position().map(|p| p.item) == Some(item(2))
    });
    let before = rig.signals();

    let x = extra(&rig, 3, QueueOrigin::Manual);
    assert_eq!(
        edit(
            &mut rig,
            1,
            vec![x.clone()],
            play_next(&[3], View::Library),
            InputSource::Mouse
        ),
        Err(PlaybackError::StaleQueue {
            expected: item(1),
            current: Some(item(2)),
        })
    );
    settle_edits(&mut rig);
    assert_eq!(rig.signals(), before, "the stale edit left no trace");
    assert_eq!(rig.ctl.diagnostics().edits_refused, 0);

    // The same edit on the real head goes through and is the only queue signal.
    edit(
        &mut rig,
        2,
        vec![x],
        play_next(&[3], View::Library),
        InputSource::Mouse,
    )
    .expect("edit on the current track");
    settle_edits(&mut rig);
    let queued = rig.of_kind(SignalKind::QueuedNext);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].recording, Some(recording(3)));
}

#[test]
fn an_edit_the_decoder_refuses_emits_nothing_and_is_counted() {
    let mut rig = Rig::new("edit-sig-refused");
    // Both tracks are tiny: with nothing rendered, B is already started in the decoder while A is
    // still the track the listener is on.
    let (a, b) = (rig.wav("a.wav", 9_600), rig.wav("b.wav", 9_600));
    let entries = vec![rig.entry(1, &a, 9_600), rig.entry(2, &b, 9_600)];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.settle();
    rig.wait_buffered(9_601);
    let x = extra(&rig, 3, QueueOrigin::Manual);

    edit(
        &mut rig,
        1,
        vec![x],
        play_next(&[3], View::Library),
        InputSource::Mouse,
    )
    .expect("the call cannot see the race");
    settle_edits(&mut rig);

    assert_eq!(rig.ctl.diagnostics().edits_refused, 1);
    assert!(
        rig.signals().is_empty(),
        "a refused edit records nothing, and nothing was heard yet: {:?}",
        kinds(&rig.signals())
    );
}

#[test]
fn auto_edits_are_applied_but_are_not_the_listeners_preference() {
    let (mut rig, entries) = playing("edit-sig-auto");
    let (x, y) = (
        extra(&rig, 3, QueueOrigin::Radio),
        extra(&rig, 4, QueueOrigin::Manual),
    );
    // The queue layer refills (Auto) and the listener then plays something next (Mouse). The
    // second is the sentinel: the engine answers in order, so once its signal is out, the Auto
    // edit has been answered too.
    edit(
        &mut rig,
        1,
        vec![x.clone(), entries[1].clone()],
        play_next(&[3], View::Radio),
        InputSource::Auto,
    )
    .expect("auto");
    rig.ctl
        .queue_later(
            vec![extra(&rig, 5, QueueOrigin::Radio)],
            View::Radio,
            InputSource::Auto,
        )
        .expect("auto later");
    edit(
        &mut rig,
        1,
        vec![y, x, entries[1].clone()],
        play_next(&[4], View::Library),
        InputSource::Mouse,
    )
    .expect("mouse");
    settle_edits(&mut rig);

    let queue: Vec<_> = rig
        .signals()
        .into_iter()
        .filter(|e| matches!(e.kind(), SignalKind::QueuedNext | SignalKind::QueuedLater))
        .collect();
    assert_eq!(queue.len(), 1, "only the listener's edit counts");
    assert_eq!(queue[0].recording, Some(recording(4)));
}

#[test]
fn an_edit_on_a_track_restored_paused_opens_the_session_like_any_other_act() {
    let mut rig = Rig::new("edit-sig-restored");
    let (a, b) = (rig.wav("a.wav", 20 * SECOND), rig.wav("b.wav", 20 * SECOND));
    let entries = vec![rig.entry(1, &a, 20 * SECOND), rig.entry(2, &b, 20 * SECOND)];
    // The shell restores the last track paused: its own commands are `Auto`.
    rig.ctl.pause(InputSource::Auto);
    rig.select(entries.clone(), 0, PlayOrigin::Resume, InputSource::Auto);
    assert!(rig.signals().is_empty());

    let x = extra(&rig, 3, QueueOrigin::Manual);
    edit(
        &mut rig,
        1,
        vec![x, entries[1].clone()],
        play_next(&[3], View::Library),
        InputSource::Mouse,
    )
    .expect("edit");
    settle_edits(&mut rig);

    let signals = rig.signals();
    assert_eq!(
        kinds(&signals),
        [SignalKind::SessionStarted, SignalKind::QueuedNext]
    );
    let SignalPayload::SessionStarted(p) = signals[0].payload else {
        panic!("kind");
    };
    assert_eq!(p.start_origin, PlayOrigin::Resume, "the play's origin");

    // The session was opened in silence, so it ends by silence like any other.
    rig.clock.advance(30 * 60_000);
    rig.ctl.poll();
    let ended = only(&rig.signals(), SignalKind::SessionEnded);
    let SignalPayload::SessionEnded(p) = ended.payload else {
        panic!("kind");
    };
    assert_eq!((p.tracks, p.minutes), (0, 0));
}

#[test]
fn an_add_with_nothing_loaded_and_no_session_is_applied_and_not_recorded() {
    let mut rig = Rig::new("edit-sig-idle");
    let x = extra(&rig, 3, QueueOrigin::Manual);
    rig.ctl
        .queue_later(vec![x], View::Library, InputSource::Mouse)
        .expect("queue later");
    assert!(rig.signals().is_empty(), "no session, nothing recorded");
    assert_eq!(rig.ctl.status().session, None, "and none was opened for it");
}

#[test]
fn an_edit_after_the_session_expired_is_applied_and_not_recorded() {
    let (mut rig, entries) = playing("edit-sig-expired");
    rig.ctl.pause(InputSource::Mouse);
    rig.clock.advance(30 * 60_000);
    rig.ctl.poll();
    assert_eq!(rig.ctl.status().session, None, "30 minutes of silence");

    let (x, y) = (
        extra(&rig, 3, QueueOrigin::Manual),
        extra(&rig, 4, QueueOrigin::Manual),
    );
    edit(
        &mut rig,
        1,
        vec![x.clone(), entries[1].clone()],
        play_next(&[3], View::Library),
        InputSource::Mouse,
    )
    .expect("edit");
    settle_edits(&mut rig);
    assert_eq!(rig.ctl.status().session, None, "the edit opened none");
    assert_eq!(rig.of_kind(SignalKind::QueuedNext).len(), 0);

    // The listener comes back: resuming starts the next session, and then edits count again.
    rig.ctl.resume(InputSource::Mouse);
    edit(
        &mut rig,
        1,
        vec![y, x, entries[1].clone()],
        play_next(&[4], View::Library),
        InputSource::Mouse,
    )
    .expect("edit in the new session");
    settle_edits(&mut rig);
    let queued = rig.of_kind(SignalKind::QueuedNext);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].recording, Some(recording(4)));
    let session = rig.ctl.status().session.expect("a new session");
    assert_eq!(queued[0].session, session);
}

#[test]
fn a_disabled_category_constructs_no_queue_signal() {
    let (mut rig, entries) = playing("edit-sig-disabled");
    let settings = rig.bus.settings().clone();
    let queued_before = rig.bus.queued_total();
    assert_eq!(queued_before, 2, "SessionStarted and PlayStarted");

    settings.set_category(SignalCategory::QueueLibrary, false);
    let x = extra(&rig, 3, QueueOrigin::Manual);
    edit(
        &mut rig,
        1,
        vec![x.clone(), entries[1].clone()],
        play_next(&[3], View::Library),
        InputSource::Mouse,
    )
    .expect("edit while off");
    rig.ctl
        .queue_later(
            vec![extra(&rig, 5, QueueOrigin::Manual)],
            View::Library,
            InputSource::Mouse,
        )
        .expect("later while off");
    settle_edits(&mut rig);
    assert_eq!(rig.bus.queued_total(), queued_before, "nothing was built");

    settings.set_category(SignalCategory::QueueLibrary, true);
    let y = extra(&rig, 4, QueueOrigin::Manual);
    edit(
        &mut rig,
        1,
        vec![y, x, entries[1].clone()],
        play_next(&[4], View::Library),
        InputSource::Mouse,
    )
    .expect("edit while on");
    settle_edits(&mut rig);
    let queued = rig.of_kind(SignalKind::QueuedNext);
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].id.seq, 2,
        "the disabled edits did not spend a sequence number"
    );
    assert_eq!(rig.bus.dropped_total(), 0);
}

#[test]
fn an_edit_that_contradicts_its_own_description_is_rejected_before_it_is_sent() {
    let (mut rig, entries) = playing("edit-sig-invalid");
    let x = extra(&rig, 3, QueueOrigin::Manual);
    let tail = vec![x, entries[1].clone()];
    // Play next must name the leading entries of the tail.
    assert!(matches!(
        edit(
            &mut rig,
            1,
            tail.clone(),
            play_next(&[2], View::Library),
            InputSource::Mouse
        ),
        Err(PlaybackError::InvalidArgument(_))
    ));
    // A reorder by zero places is not a reorder; a moved item must be in the tail.
    for bad in [
        QueueEdit::Reorder {
            item: item(3),
            distance: 0,
        },
        QueueEdit::Remove { items: vec![] },
        QueueEdit::Remove {
            items: vec![item(2)],
        },
    ] {
        assert!(matches!(
            edit(&mut rig, 1, tail.clone(), bad, InputSource::Mouse),
            Err(PlaybackError::InvalidArgument(_))
        ));
    }
    assert_eq!(
        edit(
            &mut rig,
            1,
            tail.clone(),
            QueueEdit::Reorder {
                item: item(77),
                distance: 1
            },
            InputSource::Mouse
        ),
        Err(PlaybackError::UnknownItem(item(77)))
    );
    assert_eq!(
        edit(
            &mut rig,
            1,
            tail,
            QueueEdit::Remove {
                items: vec![item(77)]
            },
            InputSource::Mouse
        ),
        Err(PlaybackError::UnknownItem(item(77)))
    );
    assert_eq!(rig.ctl.diagnostics().edits_pending, 0, "none was sent");
    assert_eq!(kinds(&rig.signals()).len(), 2, "no queue signal");
}

// ---- P-0112: next and previous carry the item they expect to leave --------------------------

/// A (half a second) then B and C: A ends by itself and B is heard, while the display, a poll
/// behind, still shows A.
fn at_a_track_boundary(tag: &str) -> (Rig, Vec<QueueEntry>) {
    let mut rig = Rig::new(tag);
    let a = rig.wav("a.wav", SECOND / 2);
    let (b, c) = (rig.wav("b.wav", 20 * SECOND), rig.wav("c.wav", 20 * SECOND));
    let entries = vec![
        rig.entry(1, &a, SECOND / 2),
        rig.entry(2, &b, 20 * SECOND),
        rig.entry(3, &c, 20 * SECOND),
    ];
    rig.select(entries.clone(), 0, PlayOrigin::Album, InputSource::Mouse);
    rig.render_until_heard();
    rig.run_until("B is playing", |r| {
        r.ctl.position().map(|p| p.item) == Some(item(2))
    });
    rig.render_exact(4_800, 480);
    (rig, entries)
}

#[test]
fn next_pressed_on_a_track_that_has_already_ended_does_nothing() {
    let (mut rig, entries) = at_a_track_boundary("edit-next-boundary");
    let before = rig.signals();
    assert_eq!(
        kinds(&before),
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::Completed,
            SignalKind::PlayStarted
        ],
        "A completed and B is heard"
    );
    let frame = rig.ctl.position().expect("playing").frame;

    // The listener pressed Next while the display still showed A: they meant to leave A, which is
    // gone. Loading C would skip B, which they have not even seen.
    rig.step_from(
        entries.clone(),
        2,
        LoadIntent::Next,
        item(1),
        InputSource::Mouse,
    )
    .expect("a no-op is not an error");
    assert!(!rig.ctl.command_pending(), "nothing was reloaded");
    rig.render_exact(4_800, 480);
    let after = rig.signals();
    assert_eq!(after, before, "no Skipped, no new play");
    let position = rig.ctl.position().expect("still playing");
    assert_eq!(position.item, item(2));
    assert!(
        position.frame >= frame + 4_800,
        "B kept playing from where it was"
    );

    // Previous is the same.
    rig.step_from(
        entries.clone(),
        0,
        LoadIntent::Previous,
        item(1),
        InputSource::Mouse,
    )
    .expect("no-op");
    assert!(!rig.ctl.command_pending());
    assert_eq!(rig.signals(), before);

    // Pressed on the track that is playing, it works as before.
    rig.step_from(entries, 2, LoadIntent::Next, item(2), InputSource::Mouse)
        .expect("next from B");
    rig.render_until_heard();
    let skipped = only(&rig.signals(), SignalKind::Skipped);
    assert_eq!(skipped.recording, Some(recording(2)));
    assert_eq!(rig.ctl.position().map(|p| p.item), Some(item(3)));
}

#[test]
fn choosing_a_track_does_not_depend_on_what_was_playing() {
    let (mut rig, entries) = at_a_track_boundary("edit-select-boundary");
    // A double-click is "play this", whatever the display showed.
    rig.ctl
        .load(LoadRequest {
            entries,
            index: 2,
            start_frame: 0,
            intent: LoadIntent::Select(PlayOrigin::Album),
            input: InputSource::Mouse,
            leaving: Some(item(1)),
        })
        .expect("select");
    rig.render_until_heard();
    assert_eq!(rig.ctl.position().map(|p| p.item), Some(item(3)));
    assert_eq!(rig.of_kind(SignalKind::Skipped).len(), 1);
}
