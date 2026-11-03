//! Tests for the Signalsmith wrapper (T-0026). No audio device, no clock, no randomness: every
//! signal is generated and the stretcher is deterministic.
//!
//! An integration test, not a unit test, because the C++ allocation probe (lloc_probe.cpp)
//! reaches the linker through `rustc-link-arg-tests`, which cargo applies to `tests/` targets only.
// The probe's one extern function needs unsafe; the lib's deny applies to test targets too.
#![allow(unsafe_code)]
#![allow(clippy::expect_used, clippy::unwrap_used)] // test code: a failed expectation is the failure message

use tinwave_playback::stretch::{
    StretchError, Stretcher, MAX_CHANNELS, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE,
};
use tinwave_test_alloc::{count_allocations, CountingAlloc};

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

extern "C" {
    /// Heap operations (allocations plus frees) the C++ side made on this thread. Provided by
    /// `alloc_probe.cpp`, which `build.rs` links into test binaries only.
    fn tw_probe_cpp_heap_ops() -> u64;
}

fn cpp_heap_ops() -> u64 {
    // SAFETY: takes no arguments, reads a thread-local counter and has no other effect.
    unsafe { tw_probe_cpp_heap_ops() }
}

const SR: u32 = 44_100;
/// The output block the tests drive the stretcher with, and the tolerance "within one block".
const BLOCK: usize = 1024;

/// A phase-0 sine, `amp * sin(2 pi f t)`, `frames` long.
fn sine(freq: f64, amp: f64, frames: usize, sample_rate: u32) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            (amp * (2.0 * std::f64::consts::PI * freq * n as f64 / f64::from(sample_rate)).sin())
                as f32
        })
        .collect()
}

/// Stereo with different tones per channel, so a channel mix-up or a bad stride cannot pass.
fn stereo_tones(left_hz: f64, right_hz: f64, frames: usize) -> Vec<f32> {
    let l = sine(left_hz, 0.5, frames, SR);
    let r = sine(right_hz, 0.25, frames, SR);
    l.iter().zip(&r).flat_map(|(&a, &b)| [a, b]).collect()
}

/// Runs `source` (followed by silence) through `st` as ADR 0006 prescribes: a fixed number of
/// output frames per call and `floor(out * rate + carry)` input frames, the remainder carried.
/// Returns `out_frames` frames of output.
fn drive(st: &mut Stretcher, source: &[f32], rate: f64, out_frames: usize) -> Vec<f32> {
    let ch = st.channels();
    let mut out = vec![0.0f32; out_frames * ch];
    let mut pos = 0usize; // source frames consumed
    let mut carry = 0.0f64;
    let mut input = vec![0.0f32; ((BLOCK as f64 * rate).ceil() as usize + 1) * ch];
    for block in out.chunks_mut(BLOCK * ch) {
        let want = (block.len() / ch) as f64 * rate + carry;
        let take = want.floor() as usize;
        carry = want - take as f64;
        let input = &mut input[..take * ch];
        input.fill(0.0);
        let have = (source.len() / ch).saturating_sub(pos).min(take);
        if have > 0 {
            input[..have * ch].copy_from_slice(&source[pos * ch..(pos + have) * ch]);
        }
        pos += take;
        st.process(input, block).expect("process");
    }
    out
}

/// Frequency of `x[i * stride + offset]` from its positive-going zero crossings between frames
/// `from` and `to`, with linear interpolation.
fn frequency(x: &[f32], stride: usize, offset: usize, from: usize, to: usize) -> f64 {
    let at = |i: usize| f64::from(x[i * stride + offset]);
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0usize;
    for i in from..to - 1 {
        if at(i) <= 0.0 && at(i + 1) > 0.0 {
            let t = i as f64 + (0.0 - at(i)) / (at(i + 1) - at(i));
            if first.is_none() {
                first = Some(t);
            }
            last = t;
            count += 1;
        }
    }
    let first = first.expect("no zero crossing");
    (count - 1) as f64 * f64::from(SR) / (last - first)
}

fn cents(measured: f64, expected: f64) -> f64 {
    1200.0 * (measured / expected).log2()
}

/// Start and end (output frames) of the stretch of `x[i * stride + offset]` whose 10 ms RMS is at
/// least half of `steady_rms`.
fn extent(x: &[f32], stride: usize, offset: usize, steady_rms: f64) -> (usize, usize) {
    let win = 441usize;
    let frames = x.len() / stride;
    let rms = |start: usize| {
        let s: f64 = (start..start + win)
            .map(|i| f64::from(x[i * stride + offset]).powi(2))
            .sum();
        (s / win as f64).sqrt()
    };
    let loud: Vec<usize> = (0..frames - win)
        .step_by(win / 4)
        .filter(|&s| rms(s) >= 0.5 * steady_rms)
        .collect();
    (
        *loud.first().expect("never loud"),
        loud.last().copied().expect("never loud") + win,
    )
}

// ---------------------------------------------------------------------------------------------
// Acceptance: round trip
// ---------------------------------------------------------------------------------------------

#[test]
fn sine_round_trip_keeps_pitch_within_2_cents_and_length_within_one_block() {
    const LEFT_HZ: f64 = 1000.0;
    const RIGHT_HZ: f64 = 440.0;
    let source_frames = 3 * SR as usize;
    let source = stereo_tones(LEFT_HZ, RIGHT_HZ, source_frames);

    for rate in [1.25, 0.75, 1.5] {
        let mut st = Stretcher::new(2, SR).expect("new");
        let lag = st.discard_frames(rate).expect("lag");
        let expected_len = (source_frames as f64 / rate).round() as usize;
        let out = drive(&mut st, &source, rate, lag + expected_len + SR as usize);
        let out = &out[lag * 2..];

        // Pitch of both channels over the steady middle, half a second in from both ends.
        let (from, to) = (SR as usize / 2, expected_len - SR as usize / 2);
        for (offset, hz) in [(0, LEFT_HZ), (1, RIGHT_HZ)] {
            let error = cents(frequency(out, 2, offset, from, to), hz);
            assert!(
                error.abs() <= 2.0,
                "rate {rate}, channel {offset}: {hz} Hz came out {error:.3} cents off"
            );
        }

        // Length: where the tone starts and stops, against the exact stretched length.
        let steady = 0.5 / 2f64.sqrt();
        let (start, end) = extent(out, 2, 0, steady);
        assert!(
            start <= BLOCK,
            "rate {rate}: the tone starts {start} frames late (lag compensation off)"
        );
        assert!(
            end.abs_diff(expected_len) <= BLOCK,
            "rate {rate}: the tone ends at frame {end}, expected {expected_len} (+- {BLOCK})"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Acceptance: no allocation after construction
// ---------------------------------------------------------------------------------------------

#[test]
fn process_flush_and_reset_allocate_nothing_after_construction() {
    let mut st = Stretcher::new(2, SR).expect("new");

    // Everything the engine will do, prepared outside the counted region.
    let source = stereo_tones(1000.0, 440.0, 2 * SR as usize);
    let silence = vec![0.0f32; 4096 * 2];
    let mut out = vec![0.0f32; 4096 * 2];
    let mut tail = vec![0.0f32; 8192 * 2];
    // (input frames, output frames): slower, faster, 1:1, empty input, empty output, big gulp.
    let shapes = [
        (768usize, 1024usize),
        (1280, 1024),
        (1024, 1024),
        (0, 512),
        (333, 0),
        (4096, 3000),
        (1, 1),
        (1536, 1024),
    ];

    let cpp_start = cpp_heap_ops();
    let (ok, rust_ops) = count_allocations(|| {
        let mut ok = true;
        st.reset();
        for round in 0..40 {
            for &(i, o) in &shapes {
                let from = (round * 977 + i) % (source.len() / 2 - i.max(1));
                let input = &source[from * 2..(from + i) * 2];
                ok &= st.process(input, &mut out[..o * 2]).is_ok();
            }
            // Loud, then silent: the silent-input paths allocate nothing either.
            ok &= st
                .process(&silence[..2048 * 2], &mut out[..1024 * 2])
                .is_ok();
        }
        ok &= st.flush(&mut tail).is_ok();
        st.reset();
        ok &= st
            .process(&source[..1280 * 2], &mut out[..1024 * 2])
            .is_ok();
        ok
    });
    let cpp_ops = cpp_heap_ops() - cpp_start;

    assert!(ok, "a call failed");
    assert_eq!(rust_ops, 0, "Rust heap operations after construction");
    assert_eq!(cpp_ops, 0, "C++ heap operations after construction");
}

#[test]
fn the_allocation_probe_counts_construction_and_drop() {
    // Guards the test above against a probe that silently counts nothing.
    let before = cpp_heap_ops();
    let st = Stretcher::new(2, SR).expect("new");
    let after_new = cpp_heap_ops();
    drop(st);
    let after_drop = cpp_heap_ops();
    assert!(
        after_new - before > 10,
        "construction: {}",
        after_new - before
    );
    assert!(
        after_drop - after_new > 10,
        "drop: {}",
        after_drop - after_new
    );
}

// ---------------------------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------------------------

fn run_twice_same_bits(rate: f64) {
    let source = stereo_tones(700.0, 1900.0, SR as usize);
    let a = drive(
        &mut Stretcher::new(2, SR).expect("new"),
        &source,
        rate,
        2 * SR as usize,
    );
    let b = drive(
        &mut Stretcher::new(2, SR).expect("new"),
        &source,
        rate,
        2 * SR as usize,
    );
    assert!(a.iter().any(|&s| s != 0.0), "silent output proves nothing");
    assert!(
        a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
        "two stretchers disagree at rate {rate}"
    );
}

#[test]
fn equal_input_gives_bit_identical_output() {
    run_twice_same_bits(1.25);
}

#[test]
fn bit_identical_even_where_the_library_draws_random_numbers() {
    // Below rate 0.5 (stretch factor over 2) Signalsmith randomises phases from its random
    // engine, which the wrapper seeds with a constant instead of `std::random_device`.
    run_twice_same_bits(0.4);
}

#[test]
fn reset_makes_a_used_stretcher_behave_like_a_new_one() {
    let first = stereo_tones(300.0, 5000.0, SR as usize);
    let second = stereo_tones(1500.0, 800.0, SR as usize);

    let mut used = Stretcher::new(2, SR).expect("new");
    drive(&mut used, &first, 0.8, SR as usize);
    used.reset();
    let after_reset = drive(&mut used, &second, 1.3, SR as usize);

    let fresh = drive(
        &mut Stretcher::new(2, SR).expect("new"),
        &second,
        1.3,
        SR as usize,
    );
    assert!(
        after_reset
            .iter()
            .zip(&fresh)
            .all(|(x, y)| x.to_bits() == y.to_bits()),
        "history survived reset()"
    );
}

// ---------------------------------------------------------------------------------------------
// Flush, latency, channels, errors, threads
// ---------------------------------------------------------------------------------------------

#[test]
fn flush_returns_the_tail_and_zero_fills_the_rest() {
    let mut st = Stretcher::new(2, SR).expect("new");
    let source = stereo_tones(1000.0, 440.0, SR as usize);
    drive(&mut st, &source, 1.0, SR as usize);

    // Longer than the analysis block, so part of the buffer lies past the tail. The sentinel
    // proves that part is written (zeroed) rather than left alone.
    let frames = 3 * st.output_latency_frames();
    let mut tail = vec![7.0f32; frames * 2];
    st.flush(&mut tail).expect("flush");

    assert!(tail.iter().all(|s| s.is_finite()));
    let head: f64 = tail[..1000 * 2].iter().map(|&s| f64::from(s).powi(2)).sum();
    assert!(head > 1.0, "the tail of a loud tone is not silent ({head})");
    assert!(
        tail[(frames - 100) * 2..].iter().all(|&s| s == 0.0),
        "the buffer past the tail was not zero-filled"
    );
}

#[test]
fn latency_matches_the_default_preset_measured_in_the_spike() {
    // ADR 0006: 120 ms at rate 1.0 (2646 + 2646 frames at 44.1 kHz), 140 ms at 0.75, 100 ms at 1.5.
    let st = Stretcher::new(2, SR).expect("new");
    assert_eq!(st.input_latency_frames(), 2646);
    assert_eq!(st.output_latency_frames(), 2646);
    assert_eq!(st.discard_frames(1.0), Ok(5292));
    assert_eq!(st.discard_frames(0.75), Ok(2646 + 3528));
    assert_eq!(st.discard_frames(1.5), Ok(2646 + 1764));

    let wide = Stretcher::new(2, 96_000).expect("new");
    assert_eq!(wide.sample_rate(), 96_000);
    assert!(wide.input_latency_frames() > 2 * st.input_latency_frames());
}

#[test]
fn discard_frames_rejects_rates_that_make_no_sense() {
    let st = Stretcher::new(2, SR).expect("new");
    for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            matches!(st.discard_frames(rate), Err(StretchError::BadRate { .. })),
            "rate {rate}"
        );
    }
}

#[test]
fn mono_and_many_channels_keep_their_own_tones() {
    let mono = sine(1000.0, 0.5, 2 * SR as usize, SR);
    let mut st = Stretcher::new(1, SR).expect("new");
    let lag = st.discard_frames(1.25).expect("lag");
    let out = drive(&mut st, &mono, 1.25, lag + SR as usize);
    let hz = frequency(&out[lag..], 1, 0, SR as usize / 4, SR as usize / 2 + 4096);
    assert!(cents(hz, 1000.0).abs() <= 2.0, "{hz}");

    // Six channels (5.1 layout), one tone per channel.
    let tones = [220.0, 330.0, 440.0, 550.0, 660.0, 770.0];
    let frames = 2 * SR as usize;
    let columns: Vec<Vec<f32>> = tones.iter().map(|&f| sine(f, 0.3, frames, SR)).collect();
    let interleaved: Vec<f32> = (0..frames)
        .flat_map(|n| columns.iter().map(move |c| c[n]))
        .collect();
    let mut st = Stretcher::new(6, SR).expect("new");
    let lag = st.discard_frames(1.25).expect("lag");
    let out = drive(&mut st, &interleaved, 1.25, lag + SR as usize);
    for (c, &f) in tones.iter().enumerate() {
        let hz = frequency(
            &out[lag * 6..],
            6,
            c,
            SR as usize / 4,
            SR as usize / 2 + 4096,
        );
        assert!(cents(hz, f).abs() <= 3.0, "channel {c}: {hz} vs {f}");
    }
}

#[test]
fn bad_configurations_are_refused() {
    for (channels, rate) in [
        (0, SR),
        (MAX_CHANNELS + 1, SR),
        (usize::MAX, SR),
        (2, 0),
        (2, MIN_SAMPLE_RATE - 1),
        (2, MAX_SAMPLE_RATE + 1),
    ] {
        assert_eq!(
            Stretcher::new(channels, rate).unwrap_err(),
            StretchError::Unsupported {
                channels,
                sample_rate: rate
            }
        );
    }
    assert!(Stretcher::new(MAX_CHANNELS, MAX_SAMPLE_RATE).is_ok());
    assert!(Stretcher::new(1, MIN_SAMPLE_RATE).is_ok());
}

#[test]
fn buffers_that_are_not_whole_frames_are_refused_and_left_untouched() {
    let mut st = Stretcher::new(2, SR).expect("new");
    let mut out = [3.0f32; 8];
    assert_eq!(
        st.process(&[0.0; 5], &mut out),
        Err(StretchError::NotWholeFrames {
            len: 5,
            channels: 2
        })
    );
    assert_eq!(
        st.process(&[0.0; 4], &mut out[..7]),
        Err(StretchError::NotWholeFrames {
            len: 7,
            channels: 2
        })
    );
    assert_eq!(
        st.flush(&mut out[..3]),
        Err(StretchError::NotWholeFrames {
            len: 3,
            channels: 2
        })
    );
    assert!(out.iter().all(|&s| s == 3.0));
}

#[test]
fn a_stretcher_can_move_to_the_decoder_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<Stretcher>();

    let source = stereo_tones(1000.0, 440.0, SR as usize);
    let mut st = Stretcher::new(2, SR).expect("new");
    let handle = std::thread::spawn(move || {
        let out = drive(&mut st, &source, 1.25, SR as usize / 2);
        out.iter().any(|&s| s != 0.0)
    });
    assert!(handle.join().expect("thread"));
}

#[test]
fn errors_say_what_is_wrong() {
    let e = StretchError::Unsupported {
        channels: 0,
        sample_rate: 44_100,
    };
    let text = e.to_string();
    assert!(
        text.contains("0 channels") && text.contains("44100"),
        "{text}"
    );
}
