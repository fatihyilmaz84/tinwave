//! Playback rate (T-0027, ADR 0006): the stretch stage in the decoder thread, flush-and-restart at
//! the audible position, the renderer's fades and counters, and the events.
//!
//! The test thread plays the audio callback (`ManualOutput`), so nothing here opens a device or
//! reads a clock for an assertion: audio positions are frames, and every wait is a bounded spin
//! on a condition. The only sleeps keep a rate change clear of the previous one's 125 ms
//! throttle interval (a lower bound on spacing, so a slow machine only makes it safer).
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::*;
use tinwave_playback::{EventKind, LeaveCause, PlaybackError, QueueItemId};

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

/// The test thread as audio callback, recording the left channel of every real frame and the
/// engine's reported position after every callback.
struct Rec {
    h: Harness,
    left: Vec<f32>,
    /// `(frames recorded so far, source frame the engine reports)` after each callback.
    trace: Vec<(usize, u64)>,
    /// Frames recorded when each rate flush was seen to complete.
    restarts: Vec<usize>,
    flushes: u64,
}

impl Rec {
    fn new(h: Harness) -> Self {
        Self {
            h,
            left: Vec::new(),
            trace: Vec::new(),
            restarts: Vec::new(),
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
        std::thread::sleep(Duration::from_millis(130));
        // Audio queued for the fade-out, and callbacks no faster than a device's while the
        // decoder answers (64 frames per millisecond is a little over real time): with an empty
        // ring the fade-out would be short, and that is not what is being tested.
        self.h.wait_buffered(RATE as usize / 2);
        let before = self.flushes;
        self.h.engine.set_rate(rate).expect("set_rate");
        for _ in 0..100_000 {
            let real = self.step(64);
            if self.flushes > before && self.h.engine.rate_stats().applied_rate == rate {
                return;
            }
            std::thread::sleep(Duration::from_micros(if real == 0 { 100 } else { 1_000 }));
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
    std::thread::sleep(Duration::from_millis(130));
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

#[test]
fn no_click_at_a_rate_step_the_largest_sample_step_is_the_steady_signals() {
    let dir = TempDir::new("rate-clicks");
    let file = steady_sine(&dir, "sine.wav", 20.0);
    let mut rec = Rec::new(Harness::new());
    rec.h
        .engine
        .load(vec![track(1, &file)], 0, 0)
        .expect("load");
    rec.h.settle();

    rec.play(RATE as usize / 2);
    for rate in [1.25, 0.75, 1.5, 1.0] {
        rec.change_rate(rate);
        rec.play(RATE as usize / 2);
    }
    assert_eq!(rec.restarts.len(), 4);

    // Everything the callback produced, fades included, joined at the (zero) ends of the silent
    // gaps. The steady sine's largest step is `0.5 * 2 pi * 440 / rate`. A stretched sine is not
    // quite a sine (the phase vocoder's ripple makes its steepest step up to 2 % larger, measured
    // over many start phases), so the bound is 3 %: a hard cut at this level would be a step
    // of 0.5, seventeen times the signal's, and a fade-out that is too short shows up in the
    // renderer's own tests.
    let steady = 0.5 * 2.0 * std::f64::consts::PI * 440.0 / f64::from(RATE);
    let (at, largest) = rec
        .left
        .windows(2)
        .enumerate()
        .map(|(i, w)| (i, f64::from((w[1] - w[0]).abs())))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
    assert!(
        largest <= steady * 1.03,
        "largest sample step {largest:.5} at frame {at} against the steady sine's {steady:.5}"
    );
    // The fades really dipped: the callback played something quiet at each of the four steps.
    for &r in &rec.restarts {
        let quiet = rec.left[r.saturating_sub(600)..r]
            .iter()
            .any(|s| s.abs() < 0.02);
        assert!(quiet, "no fade-out/in dip before frame {r}");
    }
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

#[test]
fn a_drag_costs_a_few_flushes_and_ends_on_the_final_value() {
    let dir = TempDir::new("rate-throttle");
    let file = steady_sine(&dir, "sine.wav", 20.0);
    let mut h = Harness::new();
    h.engine.load(vec![track(1, &file)], 0, 0).expect("load");
    h.settle();
    h.render_exact(4_800, 480);
    h.sink.take();

    // A slider drag: a new value every 10 ms for about half a second, the callback running
    // all along. Without the throttle each would restart the stream.
    let begun = Instant::now();
    let steps = 48;
    for i in 1..=steps {
        std::thread::sleep(Duration::from_millis(10));
        h.engine
            .set_rate(1.0 + 0.5 * f64::from(i) / f64::from(steps))
            .expect("set_rate");
        h.render_block(480);
    }
    let dragged = begun.elapsed();
    assert_eq!(h.engine.rate(), 1.5);
    // Keep playing until the final value is the one being played.
    for _ in 0..100_000 {
        let (_, real) = h.render_block(480);
        if h.engine.rate_stats().applied_rate == 1.5 && real > 0 {
            break;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    let stats = h.engine.rate_stats();
    assert_eq!(stats.applied_rate, 1.5, "the final value was dropped");
    assert_eq!(stats.effective_rate, 1.5);

    // One immediate flush, then at most one per 125 ms, and the closing one.
    let allowed = 2 + (dragged.as_millis() / 125) as u64;
    assert!(
        (2..=allowed).contains(&stats.flushes),
        "{} flushes for {steps} steps over {dragged:?} (2 to {allowed} expected)",
        stats.flushes,
    );
    // But every step is reported, once.
    let changes = h
        .sink
        .take()
        .into_iter()
        .filter(|k| matches!(k, EventKind::RateChanged { .. }))
        .count();
    assert_eq!(changes, steps as usize);
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
    std::thread::sleep(Duration::from_millis(130));
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
    // Straight after: inside the interval, so it waits.
    h.engine.set_rate(0.75).expect("set_rate");
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
