//! The controller on its own thread: the cadence runs without the test driving it, commands
//! answer with the engine's result, and concurrent callers lose nothing.
#![allow(clippy::expect_used)] // test code: a failed expectation is the failure message

mod common;
mod controller_support;

use std::sync::Arc;
use std::time::Duration;

use controller_support::{entry, recording, T0};
use tinwave_core::{IdGenerator, ManualClock, SeededRng, TzOffsetMin, UtcMillis};
use tinwave_playback::{
    ControllerConfig, ControllerHandle, FixedTimezone, FsOpener, LoadIntent, LoadRequest,
    ManualHandle, ManualOutput, PlayState, PlaybackError, SourceOpener,
};
use tinwave_signals::{
    BusConfig, CounterRow, InputSource, PlayOrigin, QueueOrigin, SignalBus, SignalDrain,
    SignalEvent, SignalKind, SignalPayload, SignalSettings, SignalSink, SignalsError,
};

use common::{sweep, write_wav, Gate, GatedOpener, TempDir, RATE};

const SECOND: usize = 48_000;

#[derive(Default)]
struct Collector(Vec<SignalEvent>);

impl SignalSink for Collector {
    fn write(&mut self, events: &[SignalEvent], _: &[CounterRow]) -> Result<(), SignalsError> {
        self.0.extend_from_slice(events);
        Ok(())
    }
}

struct Threaded {
    handle: ControllerHandle,
    out: ManualHandle,
    clock: ManualClock,
    bus: SignalBus,
    drain: SignalDrain,
    dir: TempDir,
}

impl Threaded {
    fn new(tag: &str) -> Self {
        Self::with_opener(tag, Arc::new(FsOpener))
    }

    fn with_opener(tag: &str, opener: Arc<dyn SourceOpener>) -> Self {
        let (output, out) = ManualOutput::new();
        let clock = ManualClock::new(UtcMillis::new(T0));
        let device = tinwave_core::DeviceId::from_parts(UtcMillis::new(T0), [4; 10]).expect("id");
        let (bus, drain) = SignalBus::channel(
            BusConfig::new(device),
            clock.clone(),
            Arc::new(SignalSettings::new()),
        )
        .expect("bus");
        let handle = ControllerHandle::spawn(
            ControllerConfig {
                sample_rate: RATE,
                device: None,
                opener,
                clock: Arc::new(clock.clone()),
                bus: bus.clone(),
                ids: IdGenerator::new(clock.clone(), SeededRng::new(7)),
                timezone: Arc::new(FixedTimezone(TzOffsetMin::UTC)),
                initial_mode: tinwave_playback::StreamMode::Shared,
                poll_interval: Duration::from_millis(2),
            },
            Box::new(output),
        )
        .expect("spawn");
        Self {
            handle,
            out,
            clock,
            bus,
            drain,
            dir: TempDir::new(tag),
        }
    }

    fn wav(&self, name: &str, frames: usize) -> std::path::PathBuf {
        let (l, r) = sweep(RATE, frames);
        let path = self.dir.path(name);
        write_wav(&path, RATE, 16, &l, &r);
        path
    }

    /// Plays the audio callback until `done` holds (bounded).
    fn play_until(&self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let mut block = vec![0.0f32; 960];
        for _ in 0..30_000 {
            if done(self) {
                return;
            }
            let real = self.out.render(&mut block, 0).unwrap_or(0);
            if real < 480 {
                std::thread::sleep(Duration::from_micros(200));
            }
        }
        panic!("never happened: {what}");
    }

    fn signals(&mut self) -> Vec<SignalEvent> {
        let mut sink = Collector::default();
        loop {
            let report = self.drain.pump(&mut sink).expect("pump");
            if !report.more {
                break;
            }
        }
        sink.0
    }
}

#[test]
fn the_thread_polls_on_its_own_and_reports_status() {
    let mut t = Threaded::new("thr-cadence");
    let a = t.wav("a.wav", SECOND / 2);
    t.handle
        .load(LoadRequest {
            entries: vec![entry(1, &a, SECOND / 2, QueueOrigin::Album)],
            index: 0,
            start_frame: 0,
            intent: LoadIntent::Select(PlayOrigin::Album),
            input: InputSource::Mouse,
        })
        .expect("load");
    let status = t.handle.status();
    assert_eq!(
        status.state,
        PlayState::Playing,
        "updated before load returned"
    );
    assert_eq!(status.recording, Some(recording(1)));

    // Nobody calls poll: the controller's own cadence sees the track end.
    t.play_until("the track completed", |t| {
        t.handle.status().state == PlayState::Idle
    });
    let mut signals = t.signals();
    let kinds: Vec<_> = signals.iter().map(SignalEvent::kind).collect();
    assert_eq!(
        kinds,
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::Completed
        ]
    );
    t.clock.advance(31 * 60_000);
    t.play_until("the session ended on the cadence", |t| {
        t.handle.status().session.is_none()
    });
    signals = t.signals();
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].kind(), SignalKind::SessionEnded);
}

#[test]
fn commands_answer_with_the_engines_result() {
    let t = Threaded::new("thr-errors");
    assert_eq!(
        t.handle.seek_ms(1_000, InputSource::Mouse),
        Err(PlaybackError::NothingLoaded)
    );
    assert_eq!(
        t.handle.set_volume(2.0, InputSource::Mouse),
        Err(PlaybackError::InvalidArgument(
            "volume_linear must be within 0.0 to 1.0"
        ))
    );
    assert_eq!(t.handle.set_volume(0.5, InputSource::Mouse), Ok(()));
}

#[test]
fn dropping_the_handle_ends_the_session() {
    let t = Threaded::new("thr-drop");
    let a = t.wav("a.wav", 10 * SECOND);
    t.handle
        .load(LoadRequest {
            entries: vec![entry(1, &a, 10 * SECOND, QueueOrigin::Manual)],
            index: 0,
            start_frame: 0,
            intent: LoadIntent::Select(PlayOrigin::Search),
            input: InputSource::Mouse,
        })
        .expect("load");
    let Threaded {
        handle,
        mut drain,
        bus: _bus,
        ..
    } = t;
    drop(handle);
    let mut sink = Collector::default();
    while drain.pump(&mut sink).expect("pump").more {}
    let kinds: Vec<_> = sink.0.iter().map(SignalEvent::kind).collect();
    assert_eq!(
        kinds,
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::SessionEnded
        ],
        "a stop in the first seconds is no outcome; quitting closes the session"
    );
}

#[test]
fn concurrent_callers_lose_nothing_and_the_engine_stays_consistent() {
    let mut t = Threaded::new("thr-stress");
    let a = t.wav("a.wav", 30 * SECOND);
    let entries = vec![entry(1, &a, 30 * SECOND, QueueOrigin::Manual)];
    t.handle
        .load(LoadRequest {
            entries: entries.clone(),
            index: 0,
            start_frame: 0,
            intent: LoadIntent::Select(PlayOrigin::Album),
            input: InputSource::Mouse,
        })
        .expect("load");

    let handle = &t.handle;
    std::thread::scope(|scope| {
        for worker in 0..4_u64 {
            let entries = entries.clone();
            scope.spawn(move || {
                for step in 0..60_u64 {
                    let input = InputSource::Keyboard;
                    let result = match (worker + step) % 6 {
                        0 => handle.set_volume(0.1 + (step % 8) as f32 / 10.0, input),
                        1 => handle.pause(input),
                        2 => handle.resume(input),
                        3 => handle.seek_ms(1_000 * (step % 20), input),
                        4 => handle.load(LoadRequest {
                            entries: entries.clone(),
                            index: 0,
                            start_frame: 0,
                            intent: LoadIntent::Next,
                            input,
                        }),
                        _ => handle.set_repeat_mode(tinwave_signals::RepeatMode::All),
                    };
                    assert!(
                        matches!(result, Ok(()) | Err(PlaybackError::Busy)),
                        "worker {worker} step {step}: {result:?}"
                    );
                    std::thread::yield_now();
                }
            });
        }
        // The test thread is the audio callback meanwhile.
        let mut block = vec![0.0f32; 960];
        for _ in 0..400 {
            let _ = t.out.render(&mut block, 0);
            std::thread::sleep(Duration::from_micros(300));
        }
    });
    t.clock.advance(5_000);
    t.play_until("the cadence settled", |t| {
        t.handle.status().state != PlayState::Idle
    });
    let signals = t.signals();

    assert_eq!(t.bus.dropped_total(), 0);
    assert_eq!(t.bus.reserved_overflows(), 0);
    let mut open = 0_i32;
    let mut session_starts = 0;
    for event in &signals {
        match event.kind() {
            SignalKind::PlayStarted => {
                assert_eq!(open, 0, "a play started before the previous one was left");
                open += 1;
            }
            SignalKind::Completed | SignalKind::Skipped => {
                assert_eq!(open, 1, "an outcome without a play");
                open -= 1;
            }
            SignalKind::SessionStarted => session_starts += 1,
            SignalKind::SessionEnded => panic!("the session cannot have ended"),
            _ => {}
        }
        if let SignalPayload::Skipped(_) = event.payload {
            assert_eq!(event.recording, Some(recording(1)));
        }
    }
    assert_eq!(session_starts, 1);
}

#[test]
fn quitting_does_not_wait_for_a_decoder_stuck_in_a_read() {
    // A track whose very first read never returns (a share that went away).
    let gate = Arc::new(Gate::default());
    let opener = Arc::new(GatedOpener {
        gate: Arc::clone(&gate),
        allow: 0,
        only: None,
    });
    let t = Threaded::with_opener("thr-stuck", opener);
    let a = t.wav("a.wav", 10 * SECOND);
    t.handle
        .load(LoadRequest {
            entries: vec![entry(1, &a, 10 * SECOND, QueueOrigin::Manual)],
            index: 0,
            start_frame: 0,
            intent: LoadIntent::Select(PlayOrigin::Search),
            input: InputSource::Mouse,
        })
        .expect("load is accepted: the decoder works in the background");
    for _ in 0..10_000 {
        if gate.blocked.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        // The decoder starts on a command once the callback acknowledges its flush.
        let _ = t.out.render(&mut [], 0);
        std::thread::sleep(Duration::from_micros(200));
    }
    assert!(
        gate.blocked.load(std::sync::atomic::Ordering::Acquire),
        "the decoder must be stuck in the read for this test to mean anything"
    );

    let Threaded {
        handle, mut drain, ..
    } = std::mem::ManuallyDrop::into_inner(std::mem::ManuallyDrop::new(t));
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let closer = std::thread::spawn(move || {
        drop(handle);
        let _ = done_tx.send(());
    });
    let returned = done_rx.recv_timeout(Duration::from_secs(5));
    // Let the decoder go so the test process does not leak a blocked thread either way.
    gate.open();
    assert!(
        returned.is_ok(),
        "dropping the handle waited for the decoder"
    );
    closer.join().expect("closer thread");

    let mut sink = Collector::default();
    while drain.pump(&mut sink).expect("pump").more {}
    let kinds: Vec<_> = sink.0.iter().map(SignalEvent::kind).collect();
    assert_eq!(
        kinds,
        [
            SignalKind::SessionStarted,
            SignalKind::PlayStarted,
            SignalKind::SessionEnded
        ],
        "the session is still closed on the way out"
    );
}
