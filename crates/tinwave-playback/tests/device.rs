//! Opt-in tests against the REAL default audio output. Every test here is `#[ignore]`d: the
//! default `cargo test` never opens a device and never makes a sound.
//!
//! Run them on purpose with
//! `cargo test -p tinwave-playback --test device -- --ignored --nocapture --test-threads=1`.
//!
//! Safety rules, kept by every test in this file:
//! - **Silent by construction.** The clip that is played is a WAV of all-zero samples, so the
//!   samples that reach the device are digital silence at any endpoint volume and at any
//!   software gain. (Playing the sweep with the software volume lowered afterwards is NOT
//!   enough: the engine starts at gain 1.0 and a load's first block can play before the volume
//!   change is seen.) The measurements only need frames to flow, which they do.
//!   `set_volume(0.0)` is still applied as a second guard, but nothing relies on it.
//! - **Short.** Under one second of playback each.
//! - **Hands off the system.** No test changes the default device, the endpoint volume or mute.
//!   (The output code has no such call; `Engine::set_volume` is the renderer's software gain.)
//! - Exclusive mode briefly takes the endpoint away from other applications while it is open
//!   (well under a second); the exclusive test is the only one that does.
//!
//! These tests measure with `Instant` because they time a physical device. That is deliberate and
//! the only wall-clock reading in the crate's tests; everything else is frame-driven.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use tinwave_core::{ManualClock, UtcMillis};
use tinwave_playback::{
    AudioOutput, Engine, EngineConfig, FsOpener, StreamInfo, StreamMode, Track,
};

/// 0.6 s of digital silence, 16-bit stereo. Silent whatever the volume, so these tests can never
/// be audible.
fn short_clip(dir: &TempDir) -> Track {
    let silence = vec![0.0f64; 28_800];
    let path = dir.path("clip.wav");
    write_wav(&path, RATE, 16, &silence, &silence);
    track(1, path)
}

struct Opened {
    engine: Engine,
    sink: Arc<CollectingSink>,
    open_time: Duration,
}

fn start_silent(output: Box<dyn AudioOutput>) -> Opened {
    let sink = Arc::new(CollectingSink::default());
    let t0 = Instant::now();
    let mut engine = Engine::start(
        EngineConfig {
            sample_rate: RATE,
            device: None,
            opener: Arc::new(FsOpener),
            sink: sink.clone(),
            clock: Arc::new(ManualClock::new(UtcMillis::new(0))),
        },
        output,
    )
    .expect("the default output opens");
    let open_time = t0.elapsed();
    // Second guard only; the clip itself is all zeros.
    engine.set_volume(0.0).expect("volume");
    Opened {
        engine,
        sink,
        open_time,
    }
}

/// Plays the clip silently and returns (load to first audible frame, position when it was seen).
fn play_until_heard(o: &mut Opened, clip: &Track) -> (Duration, u64) {
    let t0 = Instant::now();
    o.engine
        .load(vec![clip.clone()], 0, 0)
        .expect("load returns at once");
    loop {
        o.engine.poll();
        if let Some(p) = o.engine.position() {
            if p.frame > 0 {
                return (t0.elapsed(), p.frame);
            }
        }
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "no audio reached the device within 3 s: {:?}",
            o.sink.kinds()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn report(label: &str, info: &StreamInfo, open: Duration, first: Duration) {
    println!(
        "{label}: open {:.1} ms | load -> first frame heard {:.1} ms | {} {:?} {:?} | period {} fr ({:.1} ms) | buffer {} fr ({:.1} ms) | stream latency {} fr ({:.1} ms) | MMCSS {} | fallback {:?} | device {:?}",
        open.as_secs_f64() * 1e3,
        first.as_secs_f64() * 1e3,
        info.backend,
        info.mode,
        info.format,
        info.period_frames,
        f64::from(info.period_frames) * 1e3 / f64::from(RATE),
        info.buffer_frames,
        f64::from(info.buffer_frames) * 1e3 / f64::from(RATE),
        info.stream_latency_frames,
        f64::from(info.stream_latency_frames) * 1e3 / f64::from(RATE),
        info.realtime_priority,
        info.fallback,
        info.device_name,
    );
}

#[cfg(windows)]
mod wasapi {
    use super::*;
    use tinwave_playback::WasapiOutput;

    #[test]
    #[ignore = "opens the real default output device (silent, < 1 s); run with --ignored"]
    fn shared_mode_cold_start_latency_and_device_reopen() {
        let dir = TempDir::new("device-wasapi-shared");
        let clip = short_clip(&dir);
        let (output, control) = WasapiOutput::new();
        let mut o = start_silent(Box::new(output));
        let (first, _) = play_until_heard(&mut o, &clip);
        let info = control.last_info().expect("the stream reported itself");
        report("wasapi shared", &info, o.open_time, first);
        assert_eq!(info.backend, "wasapi");
        assert_eq!(info.mode, StreamMode::Shared);
        assert!(info.buffer_frames > 0 && info.period_frames > 0);

        // Re-open on the same default device mid-stream; playback resumes at the same frame.
        std::thread::sleep(Duration::from_millis(100));
        o.engine.poll();
        let before = o.engine.position().expect("playing");
        let t0 = Instant::now();
        o.engine.switch_device(None).expect("re-open the default");
        loop {
            o.engine.poll();
            if !o.engine.command_pending() {
                break;
            }
            assert!(t0.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
        let after = o.engine.position().expect("resumed");
        println!(
            "wasapi shared: re-open took {:.1} ms; position {} -> {} frames",
            t0.elapsed().as_secs_f64() * 1e3,
            before.frame,
            after.frame
        );
        // The stream resumes from the heard frame (not from the start).
        assert!(
            after.frame + 4_800 >= before.frame,
            "{before:?} -> {after:?}"
        );
        assert!(!o.sink.kinds().iter().any(|k| matches!(
            k,
            tinwave_playback::EventKind::TrackFailed { .. }
                | tinwave_playback::EventKind::OutputLost
        )));
    }

    #[test]
    #[ignore = "opens the real default output device EXCLUSIVELY (silent, < 1 s); run with --ignored"]
    fn exclusive_mode_opens_with_an_integer_format_or_says_why_not() {
        let dir = TempDir::new("device-wasapi-exclusive");
        let clip = short_clip(&dir);
        let (output, control) = WasapiOutput::new();
        control.set_exclusive(true);
        let mut o = start_silent(Box::new(output));
        let (first, _) = play_until_heard(&mut o, &clip);
        let info = control.last_info().expect("the stream reported itself");
        report("wasapi exclusive", &info, o.open_time, first);
        match info.mode {
            StreamMode::Exclusive => {
                assert!(
                    !matches!(info.format, tinwave_playback::SampleFormat::F32),
                    "exclusive mode must use an integer format"
                );
                assert_eq!(info.fallback, None);
            }
            StreamMode::Shared => {
                let why = info.fallback.expect("a refusal is never silent");
                assert!(why.contains("exclusive mode refused"), "{why}");
            }
        }
    }

    #[test]
    #[ignore = "opens the real default output device (silent, < 1 s); run with --ignored"]
    fn shared_mode_reports_the_mmcss_boost_or_the_reason() {
        let dir = TempDir::new("device-wasapi-mmcss");
        let clip = short_clip(&dir);
        let (output, control) = WasapiOutput::new();
        let mut o = start_silent(Box::new(output));
        let _ = play_until_heard(&mut o, &clip);
        let info = control.last_info().expect("the stream reported itself");
        println!(
            "wasapi MMCSS boost: {} ({:?})",
            info.realtime_priority, info.fallback
        );
        assert!(info.realtime_priority || info.fallback.is_some());
    }
}

mod generic {
    use super::*;
    use tinwave_playback::CpalOutput;

    #[test]
    #[ignore = "opens the real default output device through cpal (silent, < 1 s); run with --ignored"]
    fn cpal_cold_start_and_latency() {
        let dir = TempDir::new("device-cpal");
        let clip = short_clip(&dir);
        let (output, control) = CpalOutput::new();
        let mut o = start_silent(Box::new(output));
        let (first, _) = play_until_heard(&mut o, &clip);
        let info = control.last_info().expect("the stream reported itself");
        report("cpal shared", &info, o.open_time, first);
        assert_eq!(info.backend, "cpal");
        assert_eq!(info.mode, StreamMode::Shared);
    }
}

/// Always runs: constructing the outputs and their handles opens nothing.
#[test]
fn creating_an_output_does_not_open_a_device() {
    let (_cpal, control) = tinwave_playback::CpalOutput::new();
    assert!(control.last_info().is_none());
    #[cfg(windows)]
    {
        let (_wasapi, control) = tinwave_playback::WasapiOutput::new();
        assert!(control.last_info().is_none());
        assert!(!control.exclusive());
        control.set_exclusive(true);
        assert!(control.exclusive());
    }
}
