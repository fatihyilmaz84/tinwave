//! Playback rate (T-0027, ADR 0006): the stretch stage in the decoder thread, flush-and-restart at
//! the audible position, the renderer's fades and counters, and the events.
//!
//! The test thread plays the audio callback (`ManualOutput`), so nothing here opens a device:
//! audio positions are frames, event times come from the harness's `ManualClock`, and every wait
//! is a bounded spin on a condition. The decoder's 125 ms throttle between rate flushes is over
//! when either the monotonic clock or that injected clock says so, so a test ends it by moving
//! the clock by hand (`Harness::clock.advance`) instead of sleeping: nothing the test does can
//! hold a flush back. What real time adds is counted in the one bound that reads a clock,
//! `a_drag_costs_...`, which bounds the flushes by `Instant` elapsed time; a slower machine only
//! widens it (so a disabled throttle is caught on a machine that is not heavily loaded).
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use symphonia::core::io::MediaSource;
use tinwave_playback::{
    EventKind, FsOpener, LeaveCause, PlaybackError, QueueItemId, SourceOpener, Track,
};

/// More than the decoder's rate-flush interval (125 ms), for the engine's manual clock.
const PAST_THROTTLE_MS: i64 = 130;

/// Click spacing in the test files, seconds.
const PERIOD: f64 = 0.25;
/// Clicks are 2 ms long; their centre is 1 ms after their start.
const CLICK_CENTRE: f64 = 0.001;

/// Quiet 440 Hz bed plus a 2 ms Hann-windowed noise click every [`PERIOD`] seconds, at `rate` Hz
/// (16-bit, both channels alike). Click `k` (from 1) is centred at `k * PERIOD + 1 ms`.
fn click_train(dir: &TempDir, name: &str, rate: u32, secs: f64) -> PathBuf {
    let n = (secs * f64::from(rate)) as usize;
    let mut left: Vec<f64> = (0..n)
        .map(|i| 0.1 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / f64::from(rate)).sin())
        .collect();
    let len = (0.002 * f64::from(rate)) as usize;
    let mut seed = 99u64;
    let mut k = 1usize;
    loop {
        let start = (k as f64 * PERIOD * f64::from(rate)) as usize;
        if start + len >= n {
            break;
        }
        for j in 0..len {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let noise = (seed >> 33) as f64 / 2_147_483_648.0 - 1.0;
            let window = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * j as f64 / len as f64).cos();
            left[start + j] += 0.8 * noise * window;
        }
        k += 1;
    }
    let path = dir.path(name);
    write_wav(&path, rate, 16, &left, &left);
    path
}

/// A steady 440 Hz sine, amplitude 0.5.
fn steady_sine(dir: &TempDir, name: &str, secs: f64) -> PathBuf {
    let n = (secs * f64::from(RATE)) as usize;
    let left: Vec<f64> = (0..n)
        .map(|i| 0.5 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / f64::from(RATE)).sin())
        .collect();
    let path = dir.path(name);
    write_wav(&path, RATE, 0, &left, &left);
    path
}

/// Centres of the clicks in `left`, as frame indexes: peaks of the smoothed magnitude of the
/// second difference (which removes the 440 Hz bed), at least 125 ms apart.
fn click_peaks(left: &[f32]) -> Vec<usize> {
    const SMOOTH: usize = 48; // 1 ms
    let mut envelope = vec![0.0f64; left.len()];
    let mut hp = vec![0.0f64; left.len()];
    for i in 2..left.len() {
        hp[i] = f64::from(left[i]) - 2.0 * f64::from(left[i - 1]) + f64::from(left[i - 2]);
    }
    let mut sum = 0.0;
    for i in 0..left.len() {
        sum += hp[i].abs();
        if i >= SMOOTH {
            sum -= hp[i - SMOOTH].abs();
        }
        // The window ends at i; centre it.
        if i >= SMOOTH / 2 {
            envelope[i - SMOOTH / 2] = sum;
        }
    }
    let top = envelope.iter().copied().fold(0.0, f64::max);
    let threshold = 0.25 * top;
    let mut peaks = Vec::new();
    let mut i = 0;
    while i < envelope.len() {
        if envelope[i] > threshold {
            let end = (i + 480).min(envelope.len());
            let best = (i..end)
                .max_by(|&a, &b| envelope[a].total_cmp(&envelope[b]))
                .unwrap_or(i);
            peaks.push(best);
            i = best + 6_000;
        } else {
            i += 1;
        }
    }
    peaks
}

/// How far from a restart a click would still be one the listener hears as the rate step: 30 ms.
const RESTART_WINDOW: usize = RATE as usize * 30 / 1000;
/// How much larger than the steady signal's largest step the largest step around a restart may
/// be when the rate moves by 2 %: the stretcher's own ripple is then well below 0.1 %, so what
/// is left is the fade. A fade of 5 ms adds at most 0.3 % to a step (the fade's slope is in
/// quadrature with the signal's), one of 2 ms up to 1.6 %.
const FADE_TOLERANCE: f64 = 0.004;
/// The same for steps of up to 50 % (1.0 to 1.5): the phase vocoder's ripple differs from one
/// restart to the next by up to about 1.5 % there, which would hide a short fade, but a click
/// (a hard cut is a step more than ten times the signal's) is still far outside.
const LARGE_STEP_TOLERANCE: f64 = 0.03;

/// The largest sample step within [`RESTART_WINDOW`] of any of `restarts` and the largest step
/// everywhere else, in `left`. The windows of `starved` restarts count for neither.
fn steps_near_and_away(left: &[f32], restarts: &[usize], starved: &[usize]) -> (f64, f64) {
    let (mut near, mut away) = (0.0f64, 0.0f64);
    let within = |list: &[usize], i: usize| list.iter().any(|&r| i.abs_diff(r) <= RESTART_WINDOW);
    for (i, pair) in left.windows(2).enumerate() {
        let step = f64::from((pair[1] - pair[0]).abs());
        if within(starved, i) {
            continue;
        }
        if within(restarts, i) {
            near = near.max(step);
        } else {
            away = away.max(step);
        }
    }
    (near, away)
}

/// No click at a rate step: the largest step within 30 ms of a restart is no larger than the
/// steady signal's in the same recording, within `tolerance`. Restarts in `starved` (see
/// [`Rec::starved`]) are left out; at least half of `restarts` must be left.
fn click_free(
    left: &[f32],
    restarts: &[usize],
    starved: &[usize],
    tolerance: f64,
) -> Result<(), String> {
    if starved.len() * 2 > restarts.len() {
        return Err(format!(
            "the decoder ran dry before {} of {} restarts: the machine is too busy to test them",
            starved.len(),
            restarts.len()
        ));
    }
    let (near, away) = steps_near_and_away(left, restarts, starved);
    if away <= 0.0 {
        return Err("no audio away from the restarts to compare with".into());
    }
    if near <= away * (1.0 + tolerance) {
        Ok(())
    } else {
        Err(format!(
            "largest sample step near a restart {near:.5} is {:.2} % above the steady signal's {away:.5}",
            (near / away - 1.0) * 100.0
        ))
    }
}

/// The test thread as audio callback, recording the left channel of every real frame and the
/// engine's reported position after every callback.
struct Rec {
    h: Harness,
    left: Vec<f32>,
    /// `(frames recorded so far, source frame the engine reports)` after each callback.
    trace: Vec<(usize, u64)>,
    /// Frames recorded when each rate flush was seen to complete.
    restarts: Vec<usize>,
    /// The restarts (as in `restarts`) that began with a ring that had run low because the
    /// decoder did not see the request in time (it was starved of CPU): the old stream had nothing
    /// left to fade out, which is the machine's doing and not what a click test is about.
    starved: Vec<usize>,
    flushes: u64,
}

impl Rec {
    fn new(h: Harness) -> Self {
        Self {
            h,
            left: Vec::new(),
            trace: Vec::new(),
            restarts: Vec::new(),
            starved: Vec::new(),
            flushes: 0,
        }
    }

    /// One callback of `frames` frames; returns the real frames it carried.
    fn step(&mut self, frames: usize) -> usize {
        let (block, real) = self.h.render_block(frames);
        self.left
            .extend(block[..real * 2].chunks_exact(2).map(|f| f[0]));
        if let Some(p) = self.h.engine.position() {
            self.trace.push((self.left.len(), p.frame));
        }
        let flushes = self.h.engine.rate_stats().flushes;
        if flushes != self.flushes {
            self.flushes = flushes;
            self.restarts.push(self.left.len());
        }
        real
    }

    /// Record `frames` more real frames in 64-frame callbacks, waiting for the decoder as needed.
    fn play(&mut self, frames: usize) {
        let target = self.left.len() + frames;
        for _ in 0..400_000 {
            if self.left.len() >= target {
                return;
            }
            let want = 64.min(target - self.left.len());
            if self.step(want) < want {
                std::thread::sleep(Duration::from_micros(100));
            }
        }
        panic!("the decoder did not deliver {frames} frames");
    }

    /// Set the rate and keep playing (fade-out and all) until the new stream is running.
    fn change_rate(&mut self, rate: f64) {
        // Clear of the previous flush's throttle interval, so this one is immediate.
        self.h.clock.advance(PAST_THROTTLE_MS);
        // Audio queued for the fade-out. Then the request is sent and the decoder is waited for
        // (not the callback): commands are taken in order and a queue edit is answered when it is
        // taken, so the answer to the first edit sent behind the rate change means the rate change
        // was taken, and the answer to a second one that the pass of the decoder's loop that
        // raises the flush request is over. The callbacks that follow cannot drain the ring while
        // the decoder is slow, and are big enough (256 frames) to hold the whole 240-frame
        // fade-out, so the first one that sees the request plays it. No pacing sleeps: the only
        // wait is for the decoder's restart, which the callbacks cannot hurry.
        self.h.wait_buffered(RATE as usize / 4);
        let before = self.flushes;
        let mut ran_low = false;
        self.h.engine.set_rate(rate).expect("set_rate");
        for _ in 0..2 {
            let edit = self
                .h
                .engine
                .replace_after(QueueItemId(1), Vec::new())
                .expect("a head to edit");
            self.h.expect_edit_applied(edit);
        }
        for _ in 0..200_000 {
            // Read before the callback: once the restart is complete the new stream's first frames
            // are in the ring, and those are not the old one running low.
            let queued = self.h.out.buffered_frames().expect("stream open");
            let real = self.step(256);
            if self.flushes > before && self.h.engine.rate_stats().applied_rate == rate {
                if ran_low {
                    // `restarts` has the entry this change just made.
                    self.starved.extend(self.restarts.last());
                }
                return;
            }
            // Draining the old stream after the fade empties the ring in one go (zero); a
            // ring running dry first passes through every level below the fade's length.
            ran_low |= (1..RATE as usize / 80).contains(&queued);
            if real == 0 {
                // Waiting for the decoder's restart.
                std::thread::sleep(Duration::from_micros(100));
            }
        }
        panic!("the decoder never applied rate {rate}");
    }

    /// The source frame the engine reports at recorded frame `f` (interpolated between callbacks).
    fn source_at(&self, f: usize) -> Option<f64> {
        let j = self.trace.partition_point(|&(frames, _)| frames < f);
        let (&(f1, p1), before) = (self.trace.get(j)?, j.checked_sub(1)?);
        let &(f0, p0) = self.trace.get(before)?;
        if f1 == f0 {
            return Some(p1 as f64);
        }
        Some(p0 as f64 + (f - f0) as f64 * (p1 as f64 - p0 as f64) / (f1 - f0) as f64)
    }
}

/// Set the rate and acknowledge the flush with empty callbacks, which consume no audio: the
/// frames played before and after are exactly the ones the test renders, with no fade-out in
/// between. (The fade-out is covered by `no_click_at_a_rate_step` and the renderer's tests.)
fn change_rate_silently(h: &mut Harness, rate: f64) {
    h.clock.advance(PAST_THROTTLE_MS);
    let before = h.engine.rate_stats().flushes;
    h.engine.set_rate(rate).expect("set_rate");
    for _ in 0..100_000 {
        h.out.render(&mut [], h.latency).expect("stream open");
        h.engine.poll();
        let stats = h.engine.rate_stats();
        if stats.flushes > before && stats.applied_rate == rate {
            return;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    panic!("the decoder never applied rate {rate}");
}

// ---------------------------------------------------------------------------------------------
// Acceptance: position accuracy through four rate changes
// ---------------------------------------------------------------------------------------------

fn position_accuracy_through_four_changes(file_rate: u32) {
    let dir = TempDir::new(&format!("rate-position-{file_rate}"));
    let file = click_train(&dir, "clicks.wav", file_rate, 22.0);
    let mut rec = Rec::new(Harness::new());
    rec.h
        .engine
        .load(vec![track(1, &file)], 0, 0)
        .expect("load");
    rec.h.settle();

    // 1.0 -> 1.25 -> 0.75 -> 1.5 -> 1.0, two seconds of output at each.
    rec.play(2 * RATE as usize);
    for rate in [1.25, 0.75, 1.5, 1.0] {
        rec.change_rate(rate);
        rec.play(2 * RATE as usize);
    }
    assert_eq!(rec.restarts.len(), 4, "four rate flushes");

    // Each click's position according to the engine against where the click is in the file.
    let frame_rate = f64::from(RATE);
    let mut errors_ms = Vec::new();
    for peak in click_peaks(&rec.left) {
        // The fades and the restart gap around a rate change garble the clicks there.
        if rec
            .restarts
            .iter()
            .any(|&r| (r as isize - peak as isize).unsigned_abs() < 2_400)
        {
            continue;
        }
        let Some(position) = rec.source_at(peak) else {
            continue;
        };
        let k = ((position - CLICK_CENTRE * frame_rate) / (PERIOD * frame_rate)).round();
        if k < 1.0 {
            continue;
        }
        let truth = k * PERIOD * frame_rate + CLICK_CENTRE * frame_rate;
        errors_ms.push((position - truth) * 1000.0 / frame_rate);
    }
    assert!(
        errors_ms.len() >= 30,
        "only {} clicks could be measured",
        errors_ms.len()
    );
    let mean = errors_ms.iter().map(|e| e.abs()).sum::<f64>() / errors_ms.len() as f64;
    let worst = errors_ms.iter().fold(0.0f64, |a, e| a.max(e.abs()));
    assert!(
        mean <= 3.0,
        "{file_rate} Hz file: mean position error {mean:.2} ms over {} clicks (worst {worst:.2} ms)",
        errors_ms.len()
    );
}

#[test]
fn position_accuracy_through_four_rate_changes_is_within_3_ms_on_a_click_train() {
    position_accuracy_through_four_changes(48_000);
}

#[test]
fn position_accuracy_holds_when_the_file_has_another_rate_than_the_stream() {
    // Stretch at the file rate, then resample: the segment rate is still in stream frames.
    position_accuracy_through_four_changes(44_100);
}

// ---------------------------------------------------------------------------------------------
// Acceptance: time-weighted rate
// ---------------------------------------------------------------------------------------------

#[test]
fn time_weighted_rate_matches_the_schedule_within_a_tenth_of_a_percent() {
    let dir = TempDir::new("rate-weighted");
    let file = click_train(&dir, "clicks.wav", RATE, 24.0);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();

    let start = h.engine.rate_stats();
    let segment = 2 * RATE as usize;
    let schedule = [1.0, 1.25, 0.75, 1.5, 1.0];
    let mut weighted = 0.0;
    for rate in schedule {
        if rate != h.engine.rate_stats().applied_rate {
            change_rate_silently(&mut h, rate);
        }
        h.render_exact(segment, 480);
        weighted += rate * segment as f64;
    }
    let nominal = weighted / (schedule.len() * segment) as f64;

    let end = h.engine.rate_stats();
    assert_eq!(end.wall_frames - start.wall_frames, (5 * segment) as u64);
    assert_eq!(end.flushes - start.flushes, 4);
    let measured = end
        .time_weighted_rate_since(&start)
        .expect("audio was played");
    assert!(
        (measured - nominal).abs() / nominal < 0.001,
        "engine says {measured}, the schedule is {nominal}"
    );

    // The same advance seen from the other side: where the engine says playback is. The marks
    // and the renderer's counters are independent books; they agree to a few frames (each
    // restart rounds the position once).
    let position = h.engine.position().expect("playing").frame as f64;
    let advanced = end.source_frames() - start.source_frames();
    assert!(
        (position - advanced).abs() <= 10.0,
        "position {position} against {advanced} source frames advanced"
    );
}

// ---------------------------------------------------------------------------------------------
// Acceptance: no click at a rate step
// ---------------------------------------------------------------------------------------------

/// Plays the steady sine and changes the rate through `rates`, `each` frames apart; returns the
/// recording and checks that every change dipped (the fades ran).
fn record_rate_steps(name: &str, rates: &[f64], each: usize) -> Rec {
    let dir = TempDir::new(name);
    let file = steady_sine(&dir, "sine.wav", 40.0);
    let mut rec = Rec::new(Harness::new());
    rec.h
        .engine
        .load(vec![track(1, &file)], 0, 0)
        .expect("load");
    rec.h.settle();
    rec.play(each);
    for (i, &rate) in rates.iter().enumerate() {
        rec.change_rate(rate);
        // A little more or less each time, up to one period of the 440 Hz sine, so that the
        // restarts meet it at different phases.
        rec.play(each + (i * 41) % 109);
    }
    assert_eq!(rec.restarts.len(), rates.len());
    // The fades really dipped: the callback played something quiet at each step.
    for &r in &rec.restarts {
        let quiet = rec.left[r.saturating_sub(600)..r]
            .iter()
            .any(|s| s.abs() < 0.02);
        assert!(quiet, "no fade-out/in dip before frame {r}");
    }
    rec
}

#[test]
fn no_click_at_a_rate_step_the_largest_sample_step_is_the_steady_signals() {
    // Forty restarts, 2 % apart: the stretcher barely colours the signal, so a fade that is too
    // short stands out against the steady audio of the same recording (see [`FADE_TOLERANCE`]).
    // The sine meets the fade at a different phase each time.
    let rates: Vec<f64> = [1.02, 1.0, 0.98, 1.0]
        .into_iter()
        .cycle()
        .take(40)
        .collect();
    let rec = record_rate_steps("rate-clicks", &rates, RATE as usize / 8);
    // Everything the callback produced, fades included, joined at the (zero) ends of the silent
    // gaps.
    if let Err(why) = click_free(&rec.left, &rec.restarts, &rec.starved, FADE_TOLERANCE) {
        panic!("{why}");
    }
}

#[test]
fn no_click_at_a_large_rate_step_the_largest_sample_step_is_the_steady_signals() {
    // A stretched sine is not quite a sine (the phase vocoder's ripple makes its steepest step a
    // little larger than `0.5 * 2 pi * 440 / rate`, and by a different amount at each restart),
    // so the yardstick is this recording's own steady stretched audio, with room for the ripple.
    let rec = record_rate_steps(
        "rate-clicks-large",
        &[1.25, 0.75, 1.5, 1.0],
        RATE as usize / 2,
    );
    if let Err(why) = click_free(&rec.left, &rec.restarts, &rec.starved, LARGE_STEP_TOLERANCE) {
        panic!("{why}");
    }
}

/// A steady sine of amplitude 0.5 with a dip at each of `at`: faded out to silence over `fade`
/// frames and back in over `fade` frames, the way the recording joins the callback's output.
fn sine_with_dips(fade: usize, at: &[usize], len: usize) -> Vec<f32> {
    let mut left: Vec<f32> = (0..len)
        .map(|i| {
            (0.5 * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / f64::from(RATE)).sin()) as f32
        })
        .collect();
    for &centre in at {
        for (i, sample) in left
            .iter_mut()
            .enumerate()
            .take(centre + fade)
            .skip(centre - fade)
        {
            *sample *= i.abs_diff(centre) as f32 / fade as f32;
        }
    }
    left
}

#[test]
fn the_click_check_passes_a_5_ms_fade_and_fails_a_2_ms_one() {
    // Forty dips, each meeting the 440 Hz sine at a different phase (2,003 frames apart).
    let restarts: Vec<usize> = (0..40).map(|k| 3_000 + k * 2_003).collect();
    let len = 3_000 + 40 * 2_003;
    let five_ms = RATE as usize / 200;
    let two_ms = RATE as usize / 500;
    click_free(
        &sine_with_dips(five_ms, &restarts, len),
        &restarts,
        &[],
        FADE_TOLERANCE,
    )
    .expect("a 5 ms fade is not a click");
    let why = click_free(
        &sine_with_dips(two_ms, &restarts, len),
        &restarts,
        &[],
        FADE_TOLERANCE,
    )
    .expect_err("a 2 ms fade is a click");
    assert!(why.contains("above the steady signal's"), "{why}");
    // Restarts the decoder was too slow for are left out, and too many of them is an error.
    let hard_cut = sine_with_dips(1, &restarts, len);
    click_free(&hard_cut, &restarts, &restarts[..], LARGE_STEP_TOLERANCE)
        .expect_err("most restarts starved");
    let cut_one = [restarts[7]];
    let mut one_cut = sine_with_dips(five_ms, &restarts, len);
    let centre = restarts[7];
    one_cut[centre] = 0.0;
    one_cut[centre - 1] = 0.5;
    assert!(click_free(&one_cut, &restarts, &[], LARGE_STEP_TOLERANCE).is_err());
    click_free(&one_cut, &restarts, &cut_one, FADE_TOLERANCE)
        .expect("a starved restart is not judged");
    // A hard cut to silence and back is far beyond even the large-step tolerance.
    let hard = sine_with_dips(1, &restarts, len);
    assert!(click_free(&hard, &restarts, &[], LARGE_STEP_TOLERANCE).is_err());
}

// ---------------------------------------------------------------------------------------------
// Acceptance: bypass at 1.0 is bit-exact
// ---------------------------------------------------------------------------------------------

#[test]
fn bypass_at_one_is_bit_exact_after_a_trip_through_the_stretcher() {
    let dir = TempDir::new("rate-bypass");
    let file = dir.path("sweep.wav");
    let (l, r) = sweep(RATE, TOTAL);
    write_wav(&file, RATE, 0, &l, &r);
    let (ref_l, ref_r) = reference(&l, &r, 0);

    let mut h = Harness::new();
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    change_rate_silently(&mut h, 1.25);
    h.render_exact(10_000, 480);
    assert!(h.engine.rate_stats().effective_rate == 1.25);
    change_rate_silently(&mut h, 1.0);

    let played = 30_000;
    let rec = h.render_exact(played, 480);
    assert_eq!(h.engine.rate_stats().effective_rate, 1.0);
    let end = h.engine.position().expect("playing").frame as usize;
    let first = end - played;
    // After the 5 ms fade-in the samples are the file's, bit for bit.
    let fade = (RATE / 200) as usize;
    let (got_l, got_r) = deinterleave(&rec);
    for i in fade..played {
        assert_eq!(
            got_l[i].to_bits(),
            ref_l[first + i].to_bits(),
            "left, frame {i}"
        );
        assert_eq!(
            got_r[i].to_bits(),
            ref_r[first + i].to_bits(),
            "right, frame {i}"
        );
    }
    // And the fade-in is a ramp from silence over exactly those frames.
    assert_eq!(got_l[0], 0.0);
    for i in 1..fade {
        let gain = i as f32 / fade as f32;
        assert!(
            (got_l[i] - ref_l[first + i] * gain).abs() < 1e-6,
            "fade-in, frame {i}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The throttle: at most one flush per interval, the final value always applied
// ---------------------------------------------------------------------------------------------

/// Keep the callback running until the decoder has applied `rate` and its audio is playing.
fn play_until_applied(h: &mut Harness, rate: f64) {
    for _ in 0..100_000 {
        let (_, real) = h.render_block(480);
        let stats = h.engine.rate_stats();
        if stats.applied_rate == rate && stats.effective_rate == rate {
            return;
        }
        if real == 0 {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
    panic!("rate {rate} was never applied: {:?}", h.engine.rate_stats());
}

/// One callback of the drag; every eighth step also waits until the decoder has taken the
/// commands sent so far and gone round its loop once. Commands are taken in order and a queue
/// edit is answered when it is taken, so the answer to an edit sent now means the rate changes
/// before it have been looked at. (The edit itself changes nothing.) Without these waits the
/// whole drag could be over before the decoder looked at it, which the throttle's bound would
/// not mind, but then a missing throttle would go unnoticed.
fn drag_step_taken(h: &mut Harness, step: u32) {
    h.render_block(480);
    if step.is_multiple_of(8) {
        let edit = h
            .engine
            .replace_after(QueueItemId(1), Vec::new())
            .expect("a head to edit");
        h.render_block(480);
        h.expect_edit_applied(edit);
    }
}

#[test]
fn a_drag_costs_one_flush_per_interval_and_ends_on_the_final_value() {
    let dir = TempDir::new("rate-throttle");
    let file = steady_sine(&dir, "sine.wav", 20.0);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.render_exact(4_800, 480);
    h.sink.take();

    // Before the first flush, so no interval of the decoder's can have begun earlier.
    let begun = Instant::now();
    // The first change is flushed at once, and starts the interval.
    let mut changes = 1;
    h.engine.set_rate(1.01).expect("set_rate");
    play_until_applied(&mut h, 1.01);
    assert_eq!(h.engine.rate_stats().flushes, 1);

    // A slider drag: a new value every 2 ms of the engine's clock, 48 of them (96 ms), the
    // callback running all along. Without the throttle each step would restart the stream.
    let steps = 48;
    for i in 1..=steps {
        h.clock.advance(2);
        h.engine
            .set_rate(if i == steps {
                1.5
            } else {
                1.01 + 0.49 * f64::from(i) / f64::from(steps)
            })
            .expect("set_rate");
        changes += 1;
        drag_step_taken(&mut h, i);
    }
    assert_eq!(h.engine.rate(), 1.5);
    // The interval ends on the injected clock: the value current then, the last one of the drag,
    // is applied.
    h.clock.advance(30);
    play_until_applied(&mut h, 1.5);

    // And again, against the interval that began with that flush: down the other way.
    for i in 1..=steps {
        h.clock.advance(2);
        h.engine
            .set_rate(if i == steps {
                0.75
            } else {
                1.5 - 0.75 * f64::from(i) / f64::from(steps)
            })
            .expect("set_rate");
        changes += 1;
        drag_step_taken(&mut h, i);
    }
    h.clock.advance(30);
    play_until_applied(&mut h, 0.75);
    let elapsed = begun.elapsed();

    // Three values were applied, so at least three flushes. Every flush after the first came at
    // least 125 ms after the one before, on one of the two clocks: at most two such gaps on the
    // injected clock (it moved 252 ms in all) and one per 125 ms of real time. A slower machine
    // only widens the bound; without the throttle each of the 96 steps could restart the stream.
    let flushes = h.engine.rate_stats().flushes;
    let allowed = 1 + 2 + (elapsed.as_millis() / 125) as u64;
    assert!(
        (3..=allowed).contains(&flushes),
        "{flushes} flushes for {} steps over {elapsed:?} (3 to {allowed} expected)",
        2 * steps,
    );
    assert_eq!(h.engine.rate(), 0.75);

    // But every step is reported, once.
    let reported = h
        .sink
        .take()
        .into_iter()
        .filter(|k| matches!(k, EventKind::RateChanged { .. }))
        .count();
    assert_eq!(reported, changes);
}

// ---------------------------------------------------------------------------------------------
// The API and its events
// ---------------------------------------------------------------------------------------------

#[test]
fn set_rate_reports_each_change_once_with_its_position_and_validates() {
    let dir = TempDir::new("rate-events");
    let file = steady_sine(&dir, "sine.wav", 10.0);
    let mut h = Harness::new();

    // Nothing loaded: applied (adopted, there is nothing to flush) and reported without a position.
    h.engine.set_rate(1.25).expect("set_rate");
    assert_eq!(
        h.sink.take(),
        vec![EventKind::RateChanged {
            from: 1.0,
            to: 1.25,
            position: None
        }]
    );
    for _ in 0..30_000 {
        if h.engine.rate_stats().applied_rate == 1.25 {
            break;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    assert_eq!(h.engine.rate_stats().applied_rate, 1.25);
    assert_eq!(h.engine.rate_stats().flushes, 0, "nothing was audible");

    // Setting the same rate again is not a change.
    h.engine.set_rate(1.25).expect("same rate");
    assert!(h.sink.take().is_empty());

    // The range is 0.75 to 1.5, inclusive; anything else (and NaN) is refused, unchanged.
    for bad in [0.7499, 1.5001, 0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            h.engine.set_rate(bad),
            Err(PlaybackError::InvalidArgument(
                "rate must be within 0.75 to 1.5"
            )),
            "rate {bad}"
        );
    }
    assert_eq!(h.engine.rate(), 1.25);
    assert!(h.sink.take().is_empty());
    h.engine.set_rate(0.75).expect("slowest");
    h.engine.set_rate(1.5).expect("fastest");
    h.sink.take();

    // With a track playing the event names where.
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.render_exact(9_600, 480);
    h.sink.take();
    h.engine.set_rate(1.0).expect("set_rate");
    let events = h.sink.take();
    let [EventKind::RateChanged { from, to, position }] = events.as_slice() else {
        panic!("expected one RateChanged, got {events:?}");
    };
    assert_eq!((*from, *to), (1.5, 1.0));
    let position = position.expect("a track is playing");
    assert_eq!(position.item, QueueItemId(1));
    // 9,600 frames at 1.5x: about 14,400 source frames in.
    assert!(
        position.frame.abs_diff(14_400) <= 40,
        "reported {}",
        position.frame
    );
}

#[test]
fn the_rate_holds_across_loads_and_a_track_is_exactly_n_over_rate_long() {
    let dir = TempDir::new("rate-length");
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);

    let mut h = Harness::new();
    h.engine.set_rate(1.25).expect("set_rate");
    // Two tracks, gapless, both stretched: each is round(n / 1.25) frames, no gap, no overlap.
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    let first = (SPLIT as f64 / 1.25).round() as usize;
    let second = ((TOTAL - SPLIT) as f64 / 1.25).round() as usize;
    h.render_exact(first + second, 480);
    h.finish();
    let kinds = h.sink.kinds();
    assert_paired(&kinds, false);
    let left: Vec<(QueueItemId, u64)> = kinds
        .iter()
        .filter_map(|k| match k {
            EventKind::TrackLeft { item, frame, .. } => Some((*item, *frame)),
            _ => None,
        })
        .collect();
    // The frames reached are in source frames: the whole of each file (a frame of rounding).
    assert_eq!(left.len(), 2);
    assert_eq!(left[0].0, QueueItemId(1));
    assert!(left[0].1.abs_diff(SPLIT as u64) <= 1, "{left:?}");
    assert_eq!(left[1].0, QueueItemId(2));
    assert!(left[1].1.abs_diff((TOTAL - SPLIT) as u64) <= 1, "{left:?}");
}

#[test]
fn stretching_a_file_of_another_rate_keeps_its_length_in_stream_frames() {
    let dir = TempDir::new("rate-length-44");
    let file = dir.path("c44.wav");
    write_sweep_piece(&file, 44_100, 0, 88_200, 2); // 2 s at 44.1 kHz
    let mut h = Harness::new();
    h.engine.set_rate(1.25).expect("set_rate");
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    // 88,200 / 1.25 = 70,560 frames at 44.1 kHz = 76,800 at 48 kHz.
    h.render_exact(76_800, 480);
    h.finish();
    let reached = h.sink.kinds().into_iter().find_map(|k| match k {
        EventKind::TrackLeft { frame, .. } => Some(frame),
        _ => None,
    });
    // 2 s of the file, in stream frames: 96,000.
    assert!(reached.expect("left").abs_diff(96_000) <= 2, "{reached:?}");
}

// ---------------------------------------------------------------------------------------------
// Seek, pause, device switch and idle changes while stretched
// ---------------------------------------------------------------------------------------------

#[test]
fn a_seek_while_stretched_lands_on_the_requested_frame() {
    let dir = TempDir::new("rate-seek");
    let file = click_train(&dir, "clicks.wav", RATE, 12.0);
    let mut rec = Rec::new(Harness::new());
    rec.h
        .engine
        .set_rate(1.25)
        .expect("set_rate before playing");
    rec.h
        .engine
        .load(vec![track(1, &file)], 0, 0)
        .expect("load");
    rec.h.settle();
    rec.play(RATE as usize / 2);

    // 100 ms before click 20: it must be heard 100 ms / 1.25 after the stream starts.
    let target = (20.0 * PERIOD - 0.1) * f64::from(RATE);
    rec.h.engine.seek(target as u64).expect("seek");
    rec.h.settle();
    rec.left.clear();
    rec.trace.clear();
    rec.play(RATE as usize / 2);
    let first = *click_peaks(&rec.left).first().expect("a click");
    let expected = (0.1 + CLICK_CENTRE) / 1.25 * f64::from(RATE);
    let error_ms = (first as f64 - expected) * 1000.0 / f64::from(RATE);
    assert!(
        error_ms.abs() <= 3.0,
        "click 20 came {error_ms:.2} ms from where a seek to 100 ms before it puts it"
    );
}

#[test]
fn a_rate_change_while_paused_keeps_the_position_and_resumes_from_it() {
    let dir = TempDir::new("rate-paused");
    let file = click_train(&dir, "clicks.wav", RATE, 12.0);
    let mut rec = Rec::new(Harness::new());
    rec.h
        .engine
        .load(vec![track(1, &file)], 0, 0)
        .expect("load");
    rec.h.settle();
    rec.play(RATE as usize);
    rec.h.engine.pause();
    rec.step(480); // the ramp down
    rec.step(480);
    let paused_at = rec.h.engine.position().expect("track").frame;

    rec.change_rate(0.75);
    rec.step(480);
    let after = rec.h.engine.position().expect("track").frame;
    assert!(
        after.abs_diff(paused_at) <= 1,
        "paused at {paused_at}, then {after} after a rate change"
    );
    assert!(rec.h.engine.is_paused());

    // Resume: the audio carries on from there at 0.75.
    rec.h.engine.resume();
    rec.left.clear();
    rec.trace.clear();
    rec.play(2 * RATE as usize);
    let end = rec.h.engine.position().expect("track").frame as f64;
    let advanced = end - after as f64;
    let expected = 0.75 * (2 * RATE) as f64;
    assert!(
        (advanced - expected).abs() <= 400.0,
        "advanced {advanced} source frames in 2 s at 0.75 (expected about {expected})"
    );
}

#[test]
fn a_device_switch_keeps_the_rate_and_the_position() {
    let dir = TempDir::new("rate-device");
    let file = click_train(&dir, "clicks.wav", RATE, 14.0);
    let mut rec = Rec::new(Harness::new());
    rec.h
        .engine
        .load(vec![track(1, &file)], 0, 0)
        .expect("load");
    rec.h.settle();
    rec.change_rate(1.5);
    rec.play(RATE as usize);
    let before = rec.h.engine.position().expect("track").frame;

    rec.h
        .engine
        .switch_device(Some("other".into()))
        .expect("switch");
    rec.h.settle();
    let after = rec.h.engine.position().expect("track").frame;
    assert!(
        after.abs_diff(before) <= 2,
        "{before} before the switch, {after} after"
    );
    assert_eq!(rec.h.engine.rate_stats().applied_rate, 1.5);

    // Clicks are 0.25 s apart in the file: 1/6 s = 8,000 frames apart at 1.5x.
    rec.left.clear();
    rec.trace.clear();
    rec.play(2 * RATE as usize);
    let peaks = click_peaks(&rec.left);
    assert!(peaks.len() >= 8, "{peaks:?}");
    for pair in peaks[1..].windows(2) {
        let gap = pair[1] - pair[0];
        assert!(gap.abs_diff(8_000) <= 100, "clicks {gap} frames apart");
    }
}

#[test]
fn the_end_of_the_queue_with_audio_still_queued_restarts_where_it_was() {
    // The decoder has finished (everything is in the ring) but little has been played: a rate
    // change must still flush and resume, not be dropped because "the queue ended".
    let dir = TempDir::new("rate-ended");
    let (l, r) = sweep(RATE, TOTAL);
    let file = dir.path("a.wav");
    write_wav(&file, RATE, 16, &l[..30_000], &r[..30_000]);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.wait_buffered(30_000);
    h.render_exact(10_000, 480);

    change_rate_silently(&mut h, 1.5);
    // 20,000 source frames are left: at 1.5x that is 13,333 frames, give or take the rounding.
    let rec = h.render_exact(13_000, 480);
    assert_eq!(rec.len(), 26_000);
    let mut played = 13_000;
    for _ in 0..30_000 {
        let (_, real) = h.render_block(480);
        played += real;
        if h.sink.kinds().contains(&EventKind::QueueEnded) {
            break;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    assert!(
        (13_330..=13_340).contains(&played),
        "{played} frames from the restart to the end"
    );
    assert_paired(&h.sink.kinds(), false);
}

// ---------------------------------------------------------------------------------------------
// Device latency: the control side still hears the old stream after the restart
// ---------------------------------------------------------------------------------------------

#[test]
fn positions_stay_continuous_through_a_rate_change_with_device_latency() {
    let dir = TempDir::new("rate-latency");
    let file = steady_sine(&dir, "sine.wav", 20.0);
    let mut h = Harness::new();
    h.latency = 2_400; // 50 ms between the callback and the speaker
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.render_exact(48_000, 240);

    let mut last = h.engine.position().expect("playing").frame;
    let before = h.engine.rate_stats().flushes;
    h.clock.advance(PAST_THROTTLE_MS);
    h.engine.set_rate(1.25).expect("set_rate");
    let mut after_restart = 0;
    for _ in 0..200_000 {
        let (_, real) = h.render_block(240);
        // The engine reports a position at every callback, and it never goes back or jumps: the
        // frames in the device's buffer are still the old stream's, heard through the old marks.
        let now = h
            .engine
            .position()
            .expect("a position at every callback")
            .frame;
        assert!(now >= last, "position went back from {last} to {now}");
        assert!(now - last <= 400, "position jumped from {last} to {now}");
        last = now;
        if h.engine.rate_stats().flushes > before {
            after_restart += 1;
        }
        if after_restart > 100 {
            break;
        }
        if real == 0 {
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    assert!(after_restart > 100, "the rate change never happened");
}

#[test]
fn a_rate_change_across_a_track_boundary_in_the_latency_window_announces_the_next_track_once() {
    // Track 1 is 14,400 frames. With 100 ms (4,800 frames) of device latency, after 15,400
    // frames have been taken the listener still hears track 1, but the rate change restarts the
    // stream inside track 2.
    let dir = TempDir::new("rate-boundary");
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..14_400], &r[..14_400]);
    write_wav(&b, RATE, 16, &l[14_400..], &r[14_400..]);

    let mut h = Harness::new();
    h.latency = 4_800;
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.settle();
    h.render_exact(15_400, 200);
    assert_eq!(h.engine.position().expect("playing").item, QueueItemId(1));

    change_rate_silently(&mut h, 1.25);
    // Not yet announced: the speaker is still 4,800 frames behind.
    assert_eq!(h.engine.position().expect("playing").item, QueueItemId(1));
    h.render_exact(10_000, 480);
    assert_eq!(h.engine.position().expect("playing").item, QueueItemId(2));

    let started: Vec<_> = h
        .sink
        .kinds()
        .into_iter()
        .filter(|k| matches!(k, EventKind::TrackStarted { .. }))
        .collect();
    assert_eq!(started.len(), 2, "{started:?}");
    let left_first = h.sink.kinds().into_iter().find_map(|k| match k {
        EventKind::TrackLeft {
            item: QueueItemId(1),
            frame,
            cause,
        } => Some((frame, cause)),
        _ => None,
    });
    let (frame, cause) = left_first.expect("track 1 was left");
    assert!(frame.abs_diff(14_400) <= 1, "left at {frame}");
    assert_eq!(cause, LeaveCause::Finished);

    // The rest of track 2 (from source frame 1,000 on) at 1.25, to the end.
    let rest = ((TOTAL - 14_400 - 1_000) as f64 / 1.25).round() as usize;
    h.render_exact(rest - 10_000, 480);
    h.finish();
    assert_paired(&h.sink.kinds(), false);
}

// ---------------------------------------------------------------------------------------------
// Corners of the decoder's loop
// ---------------------------------------------------------------------------------------------

#[test]
fn a_deferred_change_is_applied_even_when_the_decoder_is_idle_at_the_end_of_the_queue() {
    // The whole file is decoded, so the decoder sleeps waiting for a command while the ring still
    // holds audio. A change that has to wait for the throttle interval must wake it.
    let dir = TempDir::new("rate-idle-deferred");
    let (l, r) = sweep(RATE, TOTAL);
    let file = dir.path("a.wav");
    write_wav(&file, RATE, 16, &l[..30_000], &r[..30_000]);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.wait_buffered(30_000);
    h.render_exact(1_000, 480);

    change_rate_silently(&mut h, 1.5);
    // Straight after: inside the interval, so it waits, with the decoder idle. (Real time ends the
    // interval too: moving the engine's clock only makes it quicker.)
    h.engine.set_rate(0.75).expect("set_rate");
    h.clock.advance(PAST_THROTTLE_MS);
    for _ in 0..30_000 {
        h.out.render(&mut [], 0).expect("stream open");
        let stats = h.engine.rate_stats();
        if stats.flushes == 2 && stats.applied_rate == 0.75 {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!(
        "the deferred change was never applied: {:?}",
        h.engine.rate_stats()
    );
}

#[test]
fn a_seek_right_after_a_rate_change_plays_at_the_new_rate_from_the_new_place() {
    let dir = TempDir::new("rate-seek-after");
    let file = steady_sine(&dir, "sine.wav", 30.0);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.render_exact(4_800, 480);

    // The seek overtakes the rate change in the decoder: its restart already uses the new rate.
    h.engine.set_rate(1.5).expect("set_rate");
    h.engine.seek(480_000).expect("seek");
    h.settle();
    let played = 2 * RATE as usize;
    h.render_exact(played, 480);
    assert_eq!(h.engine.rate_stats().applied_rate, 1.5);
    let position = h.engine.position().expect("playing").frame as f64;
    // Two seconds of output at 1.5x: three seconds of the file past the seek target.
    assert!(
        (position - (480_000.0 + 1.5 * played as f64)).abs() <= 10.0,
        "at {position}"
    );
}

#[test]
fn rate_stats_give_the_time_weighted_rate_between_two_snapshots() {
    use tinwave_playback::RateStats;
    let at = |wall_frames: u64, source_frames: f64| RateStats {
        wall_frames,
        source_micro_frames: (source_frames * 1e6) as u64,
        effective_rate: 1.0,
        applied_rate: 1.0,
        flushes: 0,
    };
    let (a, b) = (at(1_000, 1_000.0), at(3_000, 4_000.0));
    assert_eq!(b.time_weighted_rate_since(&a), Some(1.5));
    assert_eq!(b.source_frames(), 4_000.0);
    assert_eq!(a.time_weighted_rate_since(&a), None, "no audio in between");
    assert_eq!(
        a.time_weighted_rate_since(&b),
        None,
        "snapshots out of order"
    );
}

#[test]
fn a_seek_past_the_end_while_stretched_is_reported_and_the_queue_moves_on() {
    let dir = TempDir::new("rate-seek-past-end");
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);

    let mut h = Harness::new();
    h.engine.set_rate(1.25).expect("set_rate");
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.render_exact(480, 480);
    h.sink.take();
    h.engine.seek(10_000_000).expect("seek is accepted");
    h.settle();
    assert!(
        h.saw(|k| matches!(
            k,
            EventKind::SeekFailed {
                item: QueueItemId(1),
                ..
            }
        )),
        "{:?}",
        h.sink.kinds()
    );
    // B plays from its start, at 1.25.
    h.render_exact(1_000, 500);
    let position = h.engine.position().expect("playing");
    assert_eq!(position.item, QueueItemId(2));
    assert!(position.frame.abs_diff(1_250) <= 2, "{}", position.frame);
    assert_eq!(h.engine.rate_stats().applied_rate, 1.25);
}

#[test]
fn a_seek_to_the_exact_end_while_stretched_finishes_the_track() {
    let dir = TempDir::new("rate-seek-end");
    let (l, r) = sweep(RATE, TOTAL);
    let (a, b) = (dir.path("a.wav"), dir.path("b.wav"));
    write_wav(&a, RATE, 16, &l[..SPLIT], &r[..SPLIT]);
    write_wav(&b, RATE, 16, &l[SPLIT..], &r[SPLIT..]);

    let mut h = Harness::new();
    h.engine.set_rate(0.75).expect("set_rate");
    h.engine
        .load(vec![track(1, &a), track(2, &b)], 0, 0)
        .expect("load");
    h.render_exact(480, 480);
    h.engine.seek(SPLIT as u64).expect("seek");
    h.settle();
    h.render_exact(1_000, 500);
    assert_eq!(h.engine.position().expect("playing").item, QueueItemId(2));
    assert!(!h.saw(|k| matches!(k, EventKind::SeekFailed { .. })));
}

// ---------------------------------------------------------------------------------------------
// A rate restart: a track that fails again
// ---------------------------------------------------------------------------------------------

/// Opens each track normally the first time and fails every later open: a file that vanished (or
/// a share that went away) between the first read and a rate restart.
#[derive(Debug, Default)]
struct VanishingOpener {
    opens: AtomicUsize,
}

impl SourceOpener for VanishingOpener {
    fn open(&self, track: &Track) -> io::Result<Box<dyn MediaSource>> {
        if self.opens.fetch_add(1, Ordering::SeqCst) == 0 {
            FsOpener.open(track)
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "the file is gone"))
        }
    }
}

fn track_failures(kinds: &[EventKind]) -> Vec<QueueItemId> {
    kinds
        .iter()
        .filter_map(|k| match k {
            EventKind::TrackFailed { item, .. } => Some(*item),
            _ => None,
        })
        .collect()
}

fn left_with(kinds: &[EventKind]) -> Vec<(QueueItemId, LeaveCause)> {
    kinds
        .iter()
        .filter_map(|k| match k {
            EventKind::TrackLeft { item, cause, .. } => Some((*item, *cause)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_track_that_cannot_be_reopened_by_a_rate_restart_fails_once_and_leaves_as_failed() {
    let dir = TempDir::new("rate-reopen-fails");
    let file = steady_sine(&dir, "sine.wav", 10.0);
    let mut h = Harness::with_opener(Arc::new(VanishingOpener::default()));
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.render_exact(4_800, 480);
    assert!(track_failures(&h.sink.kinds()).is_empty());

    change_rate_silently(&mut h, 1.25);
    h.finish();

    let kinds = h.sink.kinds();
    assert_eq!(track_failures(&kinds), vec![QueueItemId(1)], "{kinds:?}");
    assert_eq!(
        left_with(&kinds),
        vec![(QueueItemId(1), LeaveCause::Failed)],
        "{kinds:?}"
    );
    assert_paired(&kinds, false);
}

#[test]
fn a_track_that_breaks_again_at_the_same_place_after_a_rate_restart_is_reported_once() {
    // 120,000 bytes of 8-byte float frames: the file breaks 15,000 frames in. The decoder reads
    // that far ahead at once, reports the failure, and the track ends there. The rate restart
    // reads it again from the audible frame and hits the same break.
    let dir = TempDir::new("rate-breaks-again");
    let file = steady_sine(&dir, "sine.wav", 10.0);
    let opener = Arc::new(BreakingOpener {
        broken: file.clone(),
        fail_after: 120_000,
    });
    let mut h = Harness::with_opener(opener);
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    // The decoder reads ahead and meets the break on its own: wait for its report.
    for _ in 0..30_000 {
        if !track_failures(&h.sink.kinds()).is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
    h.render_exact(4_800, 480);
    assert_eq!(track_failures(&h.sink.kinds()), vec![QueueItemId(1)]);

    change_rate_silently(&mut h, 1.25);
    // The rest of the track at 1.25: about 8,000 frames. Play it out.
    h.run_until("the queue ended", |h| {
        h.saw(|k| *k == EventKind::QueueEnded)
    });

    let kinds = h.sink.kinds();
    assert_eq!(
        track_failures(&kinds),
        vec![QueueItemId(1)],
        "one report for one failing item: {kinds:?}"
    );
    assert_eq!(
        left_with(&kinds),
        vec![(QueueItemId(1), LeaveCause::Failed)],
        "{kinds:?}"
    );
    assert_paired(&kinds, false);
}
