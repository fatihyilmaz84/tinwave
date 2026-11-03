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
//!   (well under a second); the two exclusive tests are the only ones that do.
//!
//! `exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported` is the one for the exclusive reopen
//! at the file rate (T-0066); `spikes/T-0003-audio-backend/PO-RUNBOOK.md` section 7 says how to run
//! it and what to listen for on a real DAC. The tests at the end of the file always run and open
//! no device.
//!
//! These tests measure with `Instant` because they time a physical device. That is deliberate and
//! the only wall-clock reading in the crate's tests; everything else is frame-driven.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use common::*;
use tinwave_core::{ManualClock, UtcMillis};
use tinwave_playback::{
    AudioOutput, Engine, EngineConfig, EventKind, FsOpener, ManualOutput, OpenRequest,
    OutputStream, PlaybackError, Renderer, StreamInfo, StreamMode, Track,
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

    /// One `AudioOutput::open` call as the engine made it.
    struct OpenLog {
        rate: u32,
        exclusive_only: bool,
        took: Duration,
        result: Result<StreamInfo, String>,
    }

    /// The real WASAPI output with every `open` timed and recorded. It changes nothing about the
    /// request or the stream.
    struct TimedOutput {
        inner: WasapiOutput,
        log: Arc<Mutex<Vec<OpenLog>>>,
    }

    impl AudioOutput for TimedOutput {
        fn open(
            &mut self,
            request: &OpenRequest<'_>,
            renderer: Renderer,
        ) -> Result<Box<dyn OutputStream>, PlaybackError> {
            let started = Instant::now();
            let result = self.inner.open(request, renderer);
            let took = started.elapsed();
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(OpenLog {
                    rate: request.sample_rate,
                    exclusive_only: request.exclusive_only,
                    took,
                    result: match &result {
                        Ok(stream) => Ok(stream.info()),
                        Err(error) => Err(error.to_string()),
                    },
                });
            result
        }

        fn wants_exclusive(&self) -> bool {
            self.inner.wants_exclusive()
        }
    }

    fn silent_file(dir: &TempDir, id: u64, rate: u32, frames: usize) -> Track {
        let silence = vec![0.0f64; frames];
        let path = dir.path(&format!("silence-{id}.wav"));
        write_wav(&path, rate, 16, &silence, &silence);
        track(id, path)
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1e3
    }

    /// T-0066: the exclusive reopen on a real endpoint. Plays a 44.1 kHz then a 96 kHz **silent**
    /// clip (0.4 s each) through the real default output in exclusive mode, and prints what each
    /// open cost and what it came back as. Run it as `PO-RUNBOOK.md` section 7 says; it asserts
    /// only what must hold whatever the endpoint does (a refusal of 96 kHz is a valid outcome, and
    /// is printed as such).
    #[test]
    #[ignore = "opens the real default output device EXCLUSIVELY, twice (silent, < 2 s); run with --ignored"]
    fn exclusive_reopen_from_44_1_to_96_khz_is_timed_and_reported() {
        const R44: u32 = 44_100;
        const R96: u32 = 96_000;
        let dir = TempDir::new("device-wasapi-exclusive-reopen");
        let a = silent_file(&dir, 1, R44, 17_640);
        let b = silent_file(&dir, 2, R96, 38_400);
        let (inner, control) = WasapiOutput::new();
        control.set_exclusive(true);
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut o = start_silent(Box::new(TimedOutput {
            inner,
            log: Arc::clone(&log),
        }));
        let first = control.last_info().expect("the stream reported itself");
        assert_eq!(
            first.mode,
            StreamMode::Exclusive,
            "the endpoint {:?} does not open {RATE} Hz exclusively, so there is no reopen to measure: {:?}",
            first.device_name,
            first.fallback
        );

        let load_at = Instant::now();
        o.engine
            .load(vec![a, b], 0, 0)
            .expect("load returns at once");
        let mut reopened_at: Vec<Instant> = Vec::new();
        let mut requested_at: Option<Instant> = None;
        let (mut first_a, mut last_a, mut first_b) = (None, None, None);
        loop {
            let now = Instant::now();
            // Looked at before the poll: the poll that reopens the stream clears it.
            if reopened_at.len() == 1 && requested_at.is_none() && o.engine.reopen_pending() {
                requested_at = Some(now);
            }
            o.engine.poll();
            let kinds = o.sink.kinds();
            let reopens = kinds
                .iter()
                .filter(|k| matches!(k, EventKind::OutputReopened { .. }))
                .count();
            let now = Instant::now();
            while reopened_at.len() < reopens {
                reopened_at.push(now);
            }
            if let Some(p) = o.engine.position().filter(|p| p.frame > 0) {
                if p.item.0 == 1 {
                    first_a.get_or_insert(now);
                    last_a = Some(now);
                } else {
                    first_b.get_or_insert(now);
                }
            }
            if kinds.iter().any(|k| matches!(k, EventKind::QueueEnded)) {
                break;
            }
            assert!(
                load_at.elapsed() < Duration::from_secs(5),
                "the queue did not end within 5 s: {kinds:?}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        let log = log.lock().unwrap_or_else(PoisonError::into_inner);
        println!(
            "wasapi exclusive reopen on {:?} (engine rate {RATE} Hz, files {R44} Hz then {R96} Hz)",
            first.device_name
        );
        for (i, entry) in log.iter().enumerate() {
            let outcome = match &entry.result {
                Ok(info) => format!(
                    "{:?} {:?} | period {} fr | buffer {} fr | fallback {:?}",
                    info.mode, info.format, info.period_frames, info.buffer_frames, info.fallback
                ),
                Err(why) => format!("REFUSED, no stream opened: {why}"),
            };
            println!(
                "  open #{i}: asked {} Hz{} took {:.1} ms -> {outcome}",
                entry.rate,
                if entry.exclusive_only {
                    " (exclusive only)"
                } else {
                    ""
                },
                ms(entry.took)
            );
        }
        let after = |from: Option<Instant>, to: Option<Instant>| match (from, to) {
            (Some(from), Some(to)) => format!("{:.1} ms", ms(to.saturating_duration_since(from))),
            _ => "not seen".to_string(),
        };
        println!(
            "  load -> first reopen announced: {} | load -> 44.1 kHz clip heard: {}",
            after(Some(load_at), reopened_at.first().copied()),
            after(Some(load_at), first_a)
        );
        println!(
            "  boundary: reopen requested -> new stream announced: {} | last frame of the first clip -> first frame of the second: {}",
            after(requested_at, reopened_at.get(1).copied()),
            after(last_a, first_b)
        );
        println!(
            "  (this loop polls every ~1 ms; the app polls every 10 ms while a reopen is pending. The ear is the judge of the gap.)"
        );
        let kinds = o.sink.kinds();
        let underruns = kinds
            .iter()
            .filter(|k| matches!(k, EventKind::Underrun { .. }))
            .count();
        println!("  underrun events: {underruns}");

        // What must hold whatever the endpoint answers.
        let reopens: Vec<_> = kinds
            .iter()
            .filter_map(|k| match k {
                EventKind::OutputReopened { info, .. } => Some(info),
                _ => None,
            })
            .collect();
        let at_96 = log
            .iter()
            .position(|e| e.rate == R96)
            .expect("the engine asked the device for the 96 kHz file's rate");
        for (i, entry) in log.iter().enumerate() {
            assert_eq!(
                entry.exclusive_only,
                entry.rate != RATE,
                "open #{i} at {} Hz: only a rate other than the engine's is exclusive-only",
                entry.rate
            );
            match &entry.result {
                Ok(info) if entry.exclusive_only => {
                    assert_eq!(info.mode, StreamMode::Exclusive, "open #{i}");
                }
                Err(why) => {
                    assert!(why.contains("exclusive mode refused"), "open #{i}: {why}");
                    let next = log.get(i + 1).expect("a refusal is followed by an open");
                    assert_eq!(next.rate, RATE, "a refusal retries at the engine rate");
                    assert!(!next.exclusive_only);
                }
                Ok(_) => {}
            }
        }
        if log[at_96].result.is_err() {
            assert!(
                reopens.iter().any(|i| i
                    .fallback
                    .as_deref()
                    .is_some_and(|f| f.contains("96000 Hz refused"))),
                "the refusal is reported: {reopens:?}"
            );
        }
        assert!(first_b.is_some(), "the 96 kHz clip was never heard");
        assert!(
            !kinds
                .iter()
                .any(|k| matches!(k, EventKind::TrackFailed { .. } | EventKind::OutputLost)),
            "{kinds:?}"
        );
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

// ---- T-0066: always run, no device (ManualOutput is the output) ------------------------------

const R96: u32 = 96_000;

/// A short silent 96 kHz file: the rate the fake device refuses below.
fn file_96k(dir: &TempDir) -> Track {
    let silence = vec![0.0f64; 9_600];
    let path = dir.path("96k.wav");
    write_wav(&path, R96, 16, &silence, &silence);
    track(1, path)
}

fn open_rates(h: &Harness) -> Vec<u32> {
    h.out.opens().into_iter().map(|(_, rate)| rate).collect()
}

fn fallbacks(h: &Harness) -> Vec<String> {
    h.sink
        .kinds()
        .into_iter()
        .filter_map(|k| match k {
            EventKind::OutputReopened { info, .. } => info.fallback,
            _ => None,
        })
        .collect()
}

/// An output that does not know `exclusive_only` (every output written before T-0066): it hands
/// back what the device gives, a shared stream, for a rate it refuses.
struct IgnoresExclusiveOnly(ManualOutput);

impl AudioOutput for IgnoresExclusiveOnly {
    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError> {
        self.0.open(
            &OpenRequest {
                exclusive_only: false,
                ..*request
            },
            renderer,
        )
    }

    fn wants_exclusive(&self) -> bool {
        self.0.wants_exclusive()
    }
}

#[test]
fn a_refused_rate_costs_one_failed_open_and_one_open_at_the_engine_rate() {
    let dir = TempDir::new("device-refused-open-count");
    // The fake device takes the engine's rate exclusively, not 96 kHz.
    let mut h = Harness::exclusive(Some(&[RATE]));
    h.engine
        .load(vec![file_96k(&dir)], 0, 0)
        .expect("load returns at once");
    h.settle();

    // Start-up at the engine rate, the refused 96 kHz attempt, the retry at the engine rate.
    assert_eq!(open_rates(&h), [RATE, R96, RATE]);
    // The 96 kHz attempt was one failed open: no stream was built for it, so it was not a shared
    // open that got thrown away. Two streams were opened in all (start-up and the retry).
    assert_eq!(h.out.refused_opens(), 1);
    assert_eq!(h.out.opens().len() - h.out.refused_opens(), 2);
    assert!(h.out.is_open(), "the retry's stream is the open one");
    let notes = fallbacks(&h);
    assert_eq!(notes.len(), 1, "reported once: {notes:?}");
    assert!(notes[0].contains("96000 Hz refused"), "{}", notes[0]);
    assert!(notes[0].contains("resampling"), "{}", notes[0]);
    // And the track plays: resampled at the engine's rate.
    let (_, real) = {
        h.wait_buffered(480);
        h.render_block(480)
    };
    assert_eq!(real, 480);
}

#[test]
fn an_accepted_rate_is_one_open_and_the_engine_rate_is_never_exclusive_only() {
    let dir = TempDir::new("device-accepted-open-count");
    let mut h = Harness::exclusive(None);
    h.engine
        .load(vec![file_96k(&dir)], 0, 0)
        .expect("load returns at once");
    h.settle();
    assert_eq!(open_rates(&h), [RATE, R96]);
    assert_eq!(h.out.refused_opens(), 0);
    assert!(fallbacks(&h).is_empty());
}

#[test]
fn an_output_that_ignores_exclusive_only_still_gets_the_refusal_handled() {
    let dir = TempDir::new("device-ignored-flag");
    let mut h = Harness::exclusive_through(Some(&[RATE]), |output| {
        Box::new(IgnoresExclusiveOnly(output))
    });
    h.engine
        .load(vec![file_96k(&dir)], 0, 0)
        .expect("load returns at once");
    h.settle();
    // The shared 96 kHz stream the device handed back was closed and the engine's rate opened;
    // the cost is the wasted open the flag exists to save.
    assert_eq!(open_rates(&h), [RATE, R96, RATE]);
    assert_eq!(h.out.refused_opens(), 0);
    let notes = fallbacks(&h);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("96000 Hz refused"), "{}", notes[0]);
}

#[test]
fn settle_works_in_exclusive_mode() {
    // A load that needs the output reopened at another rate is carried out only once the engine
    // is polled: `settle` must poll, or it waits for ever (it gives up with a panic after ~3 s).
    let dir = TempDir::new("device-settle-exclusive");
    let mut h = Harness::exclusive(None);
    h.engine
        .load(vec![file_96k(&dir)], 0, 0)
        .expect("load returns at once");
    h.settle();
    assert!(!h.engine.command_pending());
    assert_eq!(open_rates(&h), [RATE, R96], "the reopen happened");
}

#[test]
fn a_device_failure_at_a_new_rate_is_not_taken_for_a_refusal() {
    // Only the refusal error means "this device does not take that rate". Any other failure of
    // the open (the device went away) ends the reopen as it did before: the output is lost, the
    // rate is not blamed, and no second open is made at the engine's rate.
    let dir = TempDir::new("device-failure-not-refusal");
    let mut h = Harness::exclusive(None);
    h.out.fail_next_open();
    h.engine
        .load(vec![file_96k(&dir)], 0, 0)
        .expect("load returns at once");
    for _ in 0..30_000 {
        h.engine.poll();
        if h.saw(|k| matches!(k, EventKind::OutputLost)) {
            break;
        }
        // The decoder needs a callback to acknowledge the flush before it asks for the reopen.
        let _ = h.out.render(&mut [], h.latency);
        std::thread::sleep(Duration::from_micros(100));
    }
    assert!(
        h.saw(|k| matches!(k, EventKind::OutputLost)),
        "{:?}",
        h.sink.kinds()
    );
    assert_eq!(open_rates(&h), [RATE], "no retry at the engine rate");
    assert_eq!(h.out.refused_opens(), 0);
    assert!(fallbacks(&h).is_empty());
}
