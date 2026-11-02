//! Support for the controller tests: a `Controller` on a `ManualOutput` with a real `SignalBus`,
//! a `ManualClock` shared by engine, bus and ids, and helpers that play the audio callback.
//!
//! No test reads the system clock: signal times come from the `ManualClock`, audio positions
//! from frames; waiting for the decoder thread is a bounded spin on buffered frames.
#![allow(dead_code, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use tinwave_core::{
    DeviceId, IdGenerator, ManualClock, RecordingId, SeededRng, SessionId, SourceId, TzOffsetMin,
    UtcMillis,
};
use tinwave_playback::{
    AudioOutput, Controller, ControllerConfig, FsOpener, LoadIntent, LoadRequest, ManualHandle,
    ManualOutput, OpenRequest, OutputStream, PlaybackError, QueueEntry, QueueItemId, Renderer,
    StreamInfo, TimezoneSource, DEFAULT_POLL_INTERVAL,
};
use tinwave_signals::{
    BusConfig, CounterRow, InputSource, PlayOrigin, QueueOrigin, RepeatMode, SignalBus,
    SignalDrain, SignalEvent, SignalKind, SignalPayload, SignalSettings, SignalSink, SignalsError,
};

use crate::common::{sweep, write_wav, TempDir, RATE};

/// 2026-11-02T00:00:00Z.
pub const T0: i64 = 1_793_577_600_000;
pub const SEED: u64 = 2026;

#[derive(Default)]
struct Collector(Vec<SignalEvent>);

impl SignalSink for Collector {
    fn write(&mut self, events: &[SignalEvent], _: &[CounterRow]) -> Result<(), SignalsError> {
        self.0.extend_from_slice(events);
        Ok(())
    }
}

/// Remembers every `session_started` call.
#[derive(Debug)]
pub struct RecordingTz {
    pub offset: TzOffsetMin,
    pub started: Mutex<Vec<TzOffsetMin>>,
}

impl TimezoneSource for RecordingTz {
    fn local_offset(&self) -> TzOffsetMin {
        self.offset
    }

    fn session_started(&self, offset: TzOffsetMin) {
        self.started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(offset);
    }
}

pub struct Rig {
    pub ctl: Controller,
    pub out: ManualHandle,
    pub clock: ManualClock,
    pub bus: SignalBus,
    pub tz: Arc<RecordingTz>,
    drain: SignalDrain,
    collected: Collector,
    taken: usize,
    first_session: SessionId,
    pub dir: TempDir,
}

impl Rig {
    pub fn new(tag: &str) -> Self {
        let (output, out) = ManualOutput::new();
        Self::with_output(tag, Box::new(output), out)
    }

    pub fn with_output(tag: &str, output: Box<dyn AudioOutput>, out: ManualHandle) -> Self {
        Self::build(tag, output, out, RepeatMode::Off)
    }

    /// A rig whose controller was started with `initial_repeat` (the mode restored at launch).
    pub fn with_repeat(tag: &str, repeat: RepeatMode) -> Self {
        let (output, out) = ManualOutput::new();
        Self::build(tag, Box::new(output), out, repeat)
    }

    fn build(
        tag: &str,
        output: Box<dyn AudioOutput>,
        out: ManualHandle,
        initial_repeat: RepeatMode,
    ) -> Self {
        let clock = ManualClock::new(UtcMillis::new(T0));
        let settings = Arc::new(SignalSettings::new());
        let device = DeviceId::from_parts(UtcMillis::new(T0), [9; 10]).expect("device id");
        let (bus, drain) =
            SignalBus::channel(BusConfig::new(device), clock.clone(), settings).expect("bus");
        let tz = Arc::new(RecordingTz {
            offset: TzOffsetMin::new(120).expect("offset"),
            started: Mutex::new(Vec::new()),
        });
        let ctl = Controller::start(
            ControllerConfig {
                sample_rate: RATE,
                device: None,
                opener: Arc::new(FsOpener),
                clock: Arc::new(clock.clone()),
                bus: bus.clone(),
                ids: IdGenerator::new(clock.clone(), SeededRng::new(SEED)),
                timezone: tz.clone(),
                initial_repeat,
                poll_interval: DEFAULT_POLL_INTERVAL,
            },
            output,
        )
        .expect("controller starts");
        Self {
            ctl,
            out,
            clock,
            bus,
            tz,
            drain,
            collected: Collector::default(),
            taken: 0,
            first_session: SessionId::generate(&mut IdGenerator::new(
                ManualClock::new(UtcMillis::new(T0)),
                SeededRng::new(SEED),
            ))
            .expect("session id"),
            dir: TempDir::new(tag),
        }
    }

    /// The session id the controller draws first: same seed, clock at its start time (the id was
    /// computed when the rig was built, so moving the clock does not change it).
    pub fn first_session_id(&self) -> SessionId {
        self.first_session
    }

    // --- files ------------------------------------------------------------------------------

    /// A 16-bit stereo WAV of `frames` frames; returns its path.
    pub fn wav(&self, name: &str, frames: usize) -> PathBuf {
        let (l, r) = sweep(RATE, frames);
        let path = self.dir.path(name);
        write_wav(&path, RATE, 16, &l, &r);
        path
    }

    /// A queue entry for `path` with deterministic identities: recording and source are `id`.
    pub fn entry(&self, id: u64, path: &std::path::Path, frames: usize) -> QueueEntry {
        entry(id, path, frames, QueueOrigin::Manual)
    }

    // --- commands ---------------------------------------------------------------------------

    pub fn select(
        &mut self,
        entries: Vec<QueueEntry>,
        index: usize,
        origin: PlayOrigin,
        input: InputSource,
    ) {
        self.ctl
            .load(LoadRequest {
                entries,
                index,
                start_frame: 0,
                intent: LoadIntent::Select(origin),
                input,
            })
            .expect("load");
    }

    pub fn load_with(
        &mut self,
        entries: Vec<QueueEntry>,
        index: usize,
        intent: LoadIntent,
        input: InputSource,
    ) -> Result<(), PlaybackError> {
        self.ctl.load(LoadRequest {
            entries,
            index,
            start_frame: 0,
            intent,
            input,
        })
    }

    // --- playing the audio callback ---------------------------------------------------------

    pub fn wait_buffered(&self, frames: usize) {
        for _ in 0..30_000 {
            if self.out.buffered_frames().expect("stream open") >= frames {
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder did not deliver {frames} frames in time");
    }

    /// Keep acknowledging until the decoder carried out the last command.
    pub fn settle(&mut self) {
        for _ in 0..30_000 {
            if !self.ctl.command_pending() {
                return;
            }
            self.out.render(&mut [], 0).expect("stream open");
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder never carried out the command");
    }

    /// Render exactly `frames` frames of real audio, then poll once.
    pub fn render_exact(&mut self, frames: usize, block: usize) {
        self.settle();
        let mut done = 0;
        while done < frames {
            let want = block.min(frames - done);
            self.wait_buffered(want);
            let mut buf = vec![0.0f32; want * 2];
            let real = self.out.render(&mut buf, 0).expect("stream open");
            assert_eq!(real, want, "callback was starved");
            done += want;
            self.ctl.poll();
        }
    }

    /// Play the first 100 ms of the current track: its first position mark, which is when the
    /// controller considers the track heard (`PlayStarted`).
    pub fn hear(&mut self) {
        self.render_exact(4_800, 480);
    }

    /// Play and poll until `done` holds, in a bounded loop.
    pub fn run_until(&mut self, what: &str, mut done: impl FnMut(&mut Rig) -> bool) {
        let mut block = vec![0.0f32; 960];
        for _ in 0..30_000 {
            self.ctl.poll();
            if done(self) {
                return;
            }
            let real = self.out.render(&mut block, 0).unwrap_or(0);
            if real < 480 {
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
        }
        panic!("never happened: {what}");
    }

    /// Play until the queue is over.
    pub fn play_to_end(&mut self) {
        self.run_until("the queue ended", |r| r.ctl.status().recording.is_none());
    }

    // --- signals ----------------------------------------------------------------------------

    /// Shut the controller down (stop, flush, end the session) and return every signal.
    pub fn shutdown(self) -> Vec<SignalEvent> {
        let Rig {
            ctl,
            mut drain,
            mut collected,
            ..
        } = self;
        ctl.shutdown();
        loop {
            let report = drain.pump(&mut collected).expect("pump");
            if !report.more {
                break;
            }
        }
        collected.0
    }

    fn pump(&mut self) {
        loop {
            let report = self.drain.pump(&mut self.collected).expect("pump");
            if !report.more {
                break;
            }
        }
    }

    /// Every signal emitted so far.
    pub fn signals(&mut self) -> Vec<SignalEvent> {
        self.pump();
        self.collected.0.clone()
    }

    /// The signals emitted since the previous call.
    pub fn new_signals(&mut self) -> Vec<SignalEvent> {
        self.pump();
        let fresh = self.collected.0[self.taken..].to_vec();
        self.taken = self.collected.0.len();
        fresh
    }

    pub fn of_kind(&mut self, kind: SignalKind) -> Vec<SignalEvent> {
        self.signals()
            .into_iter()
            .filter(|e| e.kind() == kind)
            .collect()
    }
}

pub fn entry(id: u64, path: &std::path::Path, frames: usize, origin: QueueOrigin) -> QueueEntry {
    QueueEntry {
        track: crate::common::track(id, path),
        recording: RecordingId::from_parts(UtcMillis::new(T0), [id as u8; 10]).expect("recording"),
        source: SourceId::from_parts(UtcMillis::new(T0), [id as u8; 10]).expect("source"),
        duration_ms: frames as u64 * 1000 / u64::from(RATE),
        origin,
    }
}

pub fn recording(id: u64) -> RecordingId {
    RecordingId::from_parts(UtcMillis::new(T0), [id as u8; 10]).expect("recording")
}

pub fn source(id: u64) -> SourceId {
    SourceId::from_parts(UtcMillis::new(T0), [id as u8; 10]).expect("source")
}

pub fn kinds(events: &[SignalEvent]) -> Vec<SignalKind> {
    events.iter().map(SignalEvent::kind).collect()
}

/// The payload of the only event of `kind`; fails when there are none or several.
pub fn only(events: &[SignalEvent], kind: SignalKind) -> SignalEvent {
    let matching: Vec<_> = events.iter().filter(|e| e.kind() == kind).collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one {kind}, got {:?}",
        kinds(events)
    );
    *matching[0]
}

pub fn item(id: u64) -> QueueItemId {
    QueueItemId(id)
}

/// An output whose streams report the next `StreamInfo` from a list (to test `OutputReopened`).
pub struct InfoOutput {
    inner: ManualOutput,
    infos: Arc<Mutex<std::collections::VecDeque<StreamInfo>>>,
}

impl InfoOutput {
    pub fn new(
        infos: Vec<StreamInfo>,
    ) -> (
        Self,
        ManualHandle,
        Arc<Mutex<std::collections::VecDeque<StreamInfo>>>,
    ) {
        let (inner, handle) = ManualOutput::new();
        let infos = Arc::new(Mutex::new(infos.into()));
        (
            Self {
                inner,
                infos: infos.clone(),
            },
            handle,
            infos,
        )
    }
}

struct InfoStream {
    inner: Box<dyn OutputStream>,
    info: StreamInfo,
}

impl OutputStream for InfoStream {
    fn is_lost(&self) -> bool {
        self.inner.is_lost()
    }

    fn info(&self) -> StreamInfo {
        self.info.clone()
    }
}

impl AudioOutput for InfoOutput {
    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError> {
        let inner = self.inner.open(request, renderer)?;
        let info = self
            .infos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(StreamInfo::unknown);
        Ok(Box::new(InfoStream { inner, info }))
    }
}

/// Matches a payload by variant for readable asserts.
pub fn payload(event: &SignalEvent) -> SignalPayload {
    event.payload
}
