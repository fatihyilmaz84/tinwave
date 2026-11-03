//! Live queue edits in the engine (`Engine::replace_after`): play next, reorder and remove change
//! what follows the playing track without interrupting it, the hand-off into an edited next item is
//! gapless, and an edit built on an out-of-date view is refused instead of being applied to the
//! wrong item.
//!
//! The reference is the generator's sweep cut into files at frames that are not aligned to any
//! codec frame, so "the edited item plays right after the current one with a 0-frame gap" is a
//! bit-exact comparison with the uncut sweep.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use common::*;
use tinwave_playback::{EditId, EventKind, LeaveCause, PlaybackError, QueueItemId, StartCause};

/// 3.1 s of sweep, cut after 2.1 s: A is long enough that the decoder (at most one second ahead of
/// what was rendered) is still inside it when the tests edit.
const LONG: usize = 150_000;
const CUT: usize = 100_777;
/// A short filler item, the content is of no interest.
const SHORT: usize = 4_800;

struct Files {
    _dir: TempDir,
    /// `[0, CUT)` of the sweep.
    a: std::path::PathBuf,
    /// `[CUT, LONG)` of the sweep: the item that must follow `a` to make the sweep whole.
    x: std::path::PathBuf,
    /// Filler items with different content.
    b: std::path::PathBuf,
    c: std::path::PathBuf,
    ref_l: Vec<f32>,
    ref_r: Vec<f32>,
}

fn files(tag: &str) -> Files {
    let dir = TempDir::new(tag);
    let (l, r) = sweep(RATE, LONG);
    let write = |name: &str, from: usize, to: usize| {
        let path = dir.path(name);
        write_wav(&path, RATE, 16, &l[from..to], &r[from..to]);
        path
    };
    let (a, x) = (write("a.wav", 0, CUT), write("x.wav", CUT, LONG));
    // Filler from the end of the sweep, so it never equals the head of the sweep by accident.
    let (b, c) = (
        write("b.wav", LONG - SHORT, LONG),
        write("c.wav", LONG - 2 * SHORT, LONG - SHORT),
    );
    let (ref_l, ref_r) = reference(&l, &r, 16);
    Files {
        _dir: dir,
        a,
        x,
        b,
        c,
        ref_l,
        ref_r,
    }
}

fn started(kinds: &[EventKind]) -> Vec<u64> {
    kinds
        .iter()
        .filter_map(|k| match k {
            EventKind::TrackStarted { item, .. } => Some(item.0),
            _ => None,
        })
        .collect()
}

fn edited(h: &Harness, edit: EditId) -> bool {
    h.saw(|k| matches!(k, EventKind::QueueEdited { edit: e, .. } if *e == edit))
}

fn refused(h: &Harness) -> Option<(EditId, PlaybackError)> {
    h.sink.kinds().into_iter().find_map(|k| match k {
        EventKind::QueueEditRejected { edit, error, .. } => Some((edit, error)),
        _ => None,
    })
}

fn nothing_was_replaced(kinds: &[EventKind]) {
    assert!(
        !kinds.iter().any(|k| matches!(
            k,
            EventKind::TrackLeft {
                cause: LeaveCause::Replaced | LeaveCause::Stopped | LeaveCause::Failed,
                ..
            }
        )),
        "an edit must not interrupt anything: {kinds:?}"
    );
}

/// Play next: the item queued behind A is swapped for X while A plays. A keeps playing without a
/// flush, and A into X is the uncut sweep, bit for bit, with no frame lost or repeated.
#[test]
fn an_edit_during_playback_neither_interrupts_nor_gaps_the_hand_off() {
    let f = files("edit-gapless");
    let mut h = Harness::new();
    h.latency = 480;
    h.engine
        .load(vec![track(1, &f.a), track(2, &f.b)], 0, 0)
        .expect("load");
    let mut rec = h.render_exact(10_000, 480);

    let edit = h
        .engine
        .replace_after(QueueItemId(1), vec![track(3, &f.x)])
        .expect("edit");
    // The current track keeps going: the very next frames continue where the last block ended.
    let position = h.engine.position().expect("playing");
    assert_eq!(position.item, QueueItemId(1));
    assert!(!h.engine.command_pending(), "an edit is not a flush");
    rec.extend(h.render_exact(LONG - 10_000, 512));
    h.run_until("QueueEdited", |h| edited(h, edit));
    h.finish();

    assert_eq!(
        max_err(&rec, &f.ref_l, &f.ref_r, 0),
        0.0,
        "A then X must be the sweep"
    );
    let (rec_l, _) = deinterleave(&rec);
    assert_eq!(
        best_lag(&rec_l, &f.ref_l, CUT, 200, 4),
        0,
        "gap at the edit"
    );
    let kinds = h.sink.kinds();
    assert_eq!(started(&kinds), [1, 3], "B never played");
    nothing_was_replaced(&kinds);
    assert_paired(&kinds, false);
}

/// Reorder: B and C swap places behind the playing A.
#[test]
fn reordering_the_tail_plays_the_new_order() {
    let f = files("edit-reorder");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &f.a), track(2, &f.b), track(3, &f.c)], 0, 0)
        .expect("load");
    let rec = h.render_exact(5_000, 480);
    let edit = h
        .engine
        .replace_after(QueueItemId(1), vec![track(3, &f.c), track(2, &f.b)])
        .expect("edit");
    h.run_until("the end", |h| {
        h.saw(|k| matches!(k, EventKind::QueueEnded)) && edited(h, edit)
    });
    let kinds = h.sink.kinds();
    assert_eq!(started(&kinds), [1, 3, 2]);
    assert_eq!(max_err(&rec, &f.ref_l, &f.ref_r, 0), 0.0);
    nothing_was_replaced(&kinds);
    assert_paired(&kinds, false);
}

/// Remove: B is taken out and C remains; removing everything lets A end the queue by itself.
#[test]
fn removing_from_the_tail_skips_it_and_removing_all_ends_after_the_current_track() {
    let f = files("edit-remove");
    let mut h = Harness::new();
    h.engine
        .load(vec![track(1, &f.a), track(2, &f.b), track(3, &f.c)], 0, 0)
        .expect("load");
    let _ = h.render_exact(2_000, 480);
    let first = h
        .engine
        .replace_after(QueueItemId(1), vec![track(3, &f.c)])
        .expect("remove one");
    wait_for(&mut h, "first verdict", |h| edited(h, first));
    let second = h
        .engine
        .replace_after(QueueItemId(1), Vec::new())
        .expect("remove all");
    h.run_until("the end", |h| {
        h.saw(|k| matches!(k, EventKind::QueueEnded)) && edited(h, second)
    });
    let kinds = h.sink.kinds();
    assert_eq!(started(&kinds), [1], "neither B nor C played");
    assert!(
        kinds.contains(&EventKind::TrackLeft {
            item: QueueItemId(1),
            frame: CUT as u64,
            cause: LeaveCause::Finished,
        }),
        "A played to its end: {kinds:?}"
    );
    assert_paired(&kinds, false);
}

/// The edit reaches the decoder after it has read A to its end, so the queue has already ended on
/// its side: A is the last item and X is added behind it ("play next" on the last track). The
/// stream is revived and continues gaplessly, and is not declared finished at A's end.
#[test]
fn an_edit_after_the_decoder_found_the_end_of_the_queue_continues_it() {
    let f = files("edit-last");
    let (l, r) = sweep(RATE, LONG);
    let dir = TempDir::new("edit-last-files");
    let (short_a, short_b) = (dir.path("sa.wav"), dir.path("sb.wav"));
    write_wav(&short_a, RATE, 16, &l[..9_600], &r[..9_600]);
    write_wav(
        &short_b,
        RATE,
        16,
        &l[9_600..9_600 + 4_800],
        &r[9_600..9_600 + 4_800],
    );
    let mut h = Harness::new();
    h.latency = 480;
    h.engine.load(vec![track(1, &short_a)], 0, 0).expect("load");
    h.settle();
    // All of A is in the ring. The decoder finds the end of the file without another read, and
    // then waits for commands; give it the moment that takes (if it is slower than that, it is
    // still inside A when the edit arrives, which is also correct and covered elsewhere).
    h.wait_buffered(9_600);
    std::thread::sleep(std::time::Duration::from_millis(200));

    let edit = h
        .engine
        .replace_after(QueueItemId(1), vec![track(2, &short_b)])
        .expect("edit");
    // Nothing is played before the decoder has answered: the renderer would run through the end
    // of A before the queue is revived, and the queue would be declared ended there.
    wait_for(&mut h, "QueueEdited", |h| edited(h, edit));
    let rec = h.render_exact(9_600 + 4_800, 480);
    h.finish();

    assert_eq!(max_err(&rec, &f.ref_l, &f.ref_r, 0), 0.0);
    let kinds = h.sink.kinds();
    assert_eq!(started(&kinds), [1, 2]);
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, EventKind::QueueEnded))
            .count(),
        1,
        "the queue ended once, after the added item: {kinds:?}"
    );
    assert!(kinds.contains(&EventKind::TrackStarted {
        item: QueueItemId(2),
        frame: 0,
        cause: StartCause::Advance,
    }));
    // The stream was not ended behind the edit's back: B is left at its own end, not at A's.
    assert!(
        kinds.contains(&EventKind::TrackLeft {
            item: QueueItemId(2),
            frame: 4_800,
            cause: LeaveCause::Finished,
        }),
        "{kinds:?}"
    );
    assert_paired(&kinds, false);
}

/// The display showed A, but the engine has already moved on to B: the call fails and nothing
/// changes, B keeps playing and C still follows it.
#[test]
fn an_edit_built_on_a_track_that_is_no_longer_current_is_refused_by_the_call() {
    let f = files("edit-stale");
    let mut h = Harness::new();
    let (l, r) = sweep(RATE, LONG);
    let dir = TempDir::new("edit-stale-files");
    let a = dir.path("a.wav");
    write_wav(&a, RATE, 16, &l[..9_600], &r[..9_600]);
    h.engine
        .load(vec![track(1, &a), track(2, &f.b), track(3, &f.c)], 0, 0)
        .expect("load");
    // A has been heard and B has started.
    let _ = h.render_exact(9_600 + 480, 480);
    assert_eq!(h.engine.position().map(|p| p.item), Some(QueueItemId(2)));

    let result = h.engine.replace_after(QueueItemId(1), vec![track(9, &f.x)]);
    assert_eq!(
        result,
        Err(PlaybackError::StaleQueue {
            expected: QueueItemId(1),
            current: Some(QueueItemId(2)),
        })
    );
    h.run_until("the end", |h| h.saw(|k| matches!(k, EventKind::QueueEnded)));
    let kinds = h.sink.kinds();
    assert_eq!(started(&kinds), [1, 2, 3], "the queue is as it was");
    assert!(refused(&h).is_none());
    assert!(!kinds
        .iter()
        .any(|k| matches!(k, EventKind::QueueEdited { .. })));
    nothing_was_replaced(&kinds);
}

/// The display is right (A is what is heard) but the decoder, which reads ahead, has already
/// started B: B's audio is in the ring and cannot be replaced. The call cannot know that, so it
/// succeeds and the decoder thread answers with a refusal; nothing is applied to the wrong item.
#[test]
fn an_edit_that_loses_the_race_with_the_decoder_is_refused_with_an_event() {
    let f = files("edit-race");
    let mut h = Harness::new();
    let (l, r) = sweep(RATE, LONG);
    let dir = TempDir::new("edit-race-files");
    let a = dir.path("a.wav");
    write_wav(&a, RATE, 16, &l[..9_600], &r[..9_600]);
    h.engine
        .load(vec![track(1, &a), track(2, &f.b)], 0, 0)
        .expect("load");
    h.settle();
    // Nothing rendered, so A is still the item being heard; B is already decoding.
    h.wait_buffered(9_600 + 1);
    assert_eq!(h.engine.position().map(|p| p.item), Some(QueueItemId(1)));

    let edit = h
        .engine
        .replace_after(QueueItemId(1), vec![track(9, &f.x)])
        .expect("the call cannot see the race");
    wait_for(&mut h, "the refusal", |h| refused(h).is_some());
    assert_eq!(
        refused(&h),
        Some((
            edit,
            PlaybackError::StaleQueue {
                expected: QueueItemId(1),
                current: Some(QueueItemId(2)),
            }
        ))
    );
    h.run_until("the end", |h| h.saw(|k| matches!(k, EventKind::QueueEnded)));
    let kinds = h.sink.kinds();
    assert_eq!(started(&kinds), [1, 2], "X never played");
    assert!(!edited(&h, edit));
    nothing_was_replaced(&kinds);
    assert_paired(&kinds, false);
}

/// The items after A all failed to open, so the queue is exhausted on the decoder's side, but A is
/// still audible and is still the item to edit behind: the last item *opened*, not the one before
/// the end of the list.
#[test]
fn an_edit_behind_a_track_whose_successors_all_failed_to_open_is_applied() {
    let f = files("edit-failed-successors");
    let (l, r) = sweep(RATE, LONG);
    let dir = TempDir::new("edit-failed-successors-files");
    let (short_a, short_b) = (dir.path("sa.wav"), dir.path("sb.wav"));
    write_wav(&short_a, RATE, 16, &l[..9_600], &r[..9_600]);
    write_wav(
        &short_b,
        RATE,
        16,
        &l[9_600..9_600 + 4_800],
        &r[9_600..9_600 + 4_800],
    );
    let missing = dir.path("missing.wav");
    let mut h = Harness::new();
    h.latency = 480;
    h.engine
        .load(
            vec![track(1, &short_a), track(2, &missing), track(3, &missing)],
            0,
            0,
        )
        .expect("load");
    h.settle();
    // Both successors failed: the decoder is at the end of the list, A still plays.
    wait_for(&mut h, "the failures", |h| {
        h.sink
            .kinds()
            .iter()
            .filter(|k| matches!(k, EventKind::TrackFailed { .. }))
            .count()
            == 2
    });
    assert_eq!(h.engine.position().map(|p| p.item), Some(QueueItemId(1)));

    let edit = h
        .engine
        .replace_after(QueueItemId(1), vec![track(4, &short_b)])
        .expect("edit");
    wait_for(&mut h, "the verdict", |h| {
        edited(h, edit) || refused(h).is_some()
    });
    assert_eq!(refused(&h), None, "A is the last item opened");
    let rec = h.render_exact(9_600 + 4_800, 480);
    h.finish();

    assert_eq!(max_err(&rec, &f.ref_l, &f.ref_r, 0), 0.0);
    assert_eq!(started(&h.sink.kinds()), [1, 4]);
}

#[test]
fn an_edit_with_nothing_playing_or_with_duplicate_ids_is_an_error_not_a_change() {
    let f = files("edit-invalid");
    let mut h = Harness::new();
    assert_eq!(
        h.engine.replace_after(QueueItemId(1), vec![track(2, &f.b)]),
        Err(PlaybackError::NothingLoaded)
    );
    h.engine
        .load(vec![track(1, &f.a), track(2, &f.b)], 0, 0)
        .expect("load");
    let _ = h.render_exact(480, 480);
    for items in [vec![track(1, &f.x)], vec![track(5, &f.x), track(5, &f.c)]] {
        assert_eq!(
            h.engine.replace_after(QueueItemId(1), items),
            Err(PlaybackError::InvalidArgument(
                "queue item ids must be unique"
            ))
        );
    }
    h.engine.stop().expect("stop");
    assert_eq!(
        h.engine.replace_after(QueueItemId(1), vec![track(5, &f.x)]),
        Err(PlaybackError::NothingLoaded)
    );
}

/// Ids must be unique in the whole queue, and the part before the head is only known to the
/// decoder thread: it refuses an edit that would repeat one.
#[test]
fn an_edit_that_repeats_an_id_of_an_earlier_item_is_refused_by_the_decoder() {
    let f = files("edit-duplicate-prefix");
    let mut h = Harness::new();
    h.engine
        // The head is long, so the decoder is certainly still inside it.
        .load(vec![track(1, &f.b), track(2, &f.a), track(3, &f.c)], 1, 0)
        .expect("load");
    h.settle();
    let edit = h
        .engine
        .replace_after(QueueItemId(2), vec![track(1, &f.x)])
        .expect("the call only knows the head");
    wait_for(&mut h, "the refusal", |h| refused(h).is_some());
    assert_eq!(
        refused(&h),
        Some((
            edit,
            PlaybackError::InvalidArgument("queue item ids must be unique")
        ))
    );
}

/// Polls (without rendering any audio) until `done`: the verdict of an edit does not need the
/// callback, and rendering meanwhile would let the playhead run ahead of the test's expectations.
fn wait_for(h: &mut Harness, what: &str, mut done: impl FnMut(&Harness) -> bool) {
    for _ in 0..30_000 {
        h.engine.poll();
        if done(h) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
    panic!("never happened: {what}");
}
