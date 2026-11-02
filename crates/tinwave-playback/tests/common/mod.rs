//! Shared test support: generated reference sweep, tiny WAV writer, a harness that plays the part
//! of the audio callback on the test thread, and comparison helpers.
//!
//! No test reads the system clock: audio positions come from frames, event times from a
//! `ManualClock`, and waiting for the decoder thread is a bounded spin on buffered frames.
#![allow(dead_code)]

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Condvar;
use std::sync::{Arc, Mutex, PoisonError};

use symphonia::core::io::MediaSource;
use tinwave_core::{ManualClock, UtcMillis};
use tinwave_playback::{
    Engine, EngineConfig, EventKind, EventSink, FsOpener, ManualHandle, ManualOutput,
    PlaybackEvent, QueueItemId, SourceOpener, Track,
};

pub const RATE: u32 = 48_000;
/// 2 s sweep, split 777 frames after 1 s: not aligned to any codec frame (1152, 4096, ...).
pub const TOTAL: usize = 96_000;
pub const SPLIT: usize = 48_777;
pub const AMP: f64 = 0.1; // -20 dBFS

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/audio")
        .join(name)
}

/// The generator's sweep (spikes/T-0003-audio-backend/src/gen.rs): exponential 200 Hz to 12 kHz,
/// left = sweep, right = half of it.
pub fn sweep(rate: u32, total: usize) -> (Vec<f64>, Vec<f64>) {
    let (f0, f1) = (200.0_f64, 12_000.0_f64);
    let t_total = total as f64 / f64::from(rate);
    let k = (f1 / f0).ln();
    let mut l = Vec::with_capacity(total);
    let mut r = Vec::with_capacity(total);
    for n in 0..total {
        let t = n as f64 / f64::from(rate);
        let phase = 2.0 * std::f64::consts::PI * f0 * t_total / k * ((t / t_total * k).exp() - 1.0);
        let s = AMP * phase.sin();
        l.push(s);
        r.push(0.5 * s);
    }
    (l, r)
}

/// What a decoder must output for a PCM source of `bits` (0 = float32): the sweep quantised the
/// way the generator does it and read back the way Symphonia does it.
pub fn reference(l: &[f64], r: &[f64], bits: u32) -> (Vec<f32>, Vec<f32>) {
    let q = |v: f64| -> f32 {
        match bits {
            16 => ((v * 32767.0).round() / 32768.0) as f32,
            24 => ((v * 8_388_607.0).round() / 8_388_608.0) as f32,
            _ => v as f32,
        }
    };
    (
        l.iter().map(|&v| q(v)).collect(),
        r.iter().map(|&v| q(v)).collect(),
    )
}

/// Minimal RIFF/WAVE writer: PCM 16/24 bit or IEEE float 32.
pub fn write_wav(path: &Path, rate: u32, bits: u32, l: &[f64], r: &[f64]) {
    write_wav_channels(path, rate, bits, 2, l, r);
}

/// As [`write_wav`] with 2 or 3 channels (the third repeats the left channel).
pub fn write_wav_channels(path: &Path, rate: u32, bits: u32, channels: u16, l: &[f64], r: &[f64]) {
    let (format_tag, bits_per_sample) = if bits == 0 {
        (3u16, 32u16)
    } else {
        (1u16, bits as u16)
    };
    let bytes_per_sample = usize::from(bits_per_sample / 8);
    let mut data = Vec::with_capacity(l.len() * 2 * bytes_per_sample);
    for (&a, &b) in l.iter().zip(r) {
        for v in [a, b, a].into_iter().take(usize::from(channels)) {
            match bits {
                16 => data.extend_from_slice(&((v * 32767.0).round() as i16).to_le_bytes()),
                24 => {
                    data.extend_from_slice(&((v * 8_388_607.0).round() as i32).to_le_bytes()[..3])
                }
                _ => data.extend_from_slice(&(v as f32).to_le_bytes()),
            }
        }
    }
    let block_align = channels * bits_per_sample / 8;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&format_tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, out).expect("write wav");
}

/// A scratch directory removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinwave-playback-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    pub fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn track(id: u64, path: impl Into<PathBuf>) -> Track {
    Track {
        id: QueueItemId(id),
        path: path.into(),
        gapless: None,
    }
}

#[derive(Debug, Default)]
pub struct CollectingSink {
    events: Mutex<Vec<PlaybackEvent>>,
}

impl CollectingSink {
    pub fn all(&self) -> Vec<PlaybackEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn kinds(&self) -> Vec<EventKind> {
        self.all().into_iter().map(|e| e.kind).collect()
    }

    pub fn take(&self) -> Vec<EventKind> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
            .into_iter()
            .map(|e| e.kind)
            .collect()
    }
}

impl EventSink for CollectingSink {
    fn emit(&self, event: PlaybackEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

pub struct Harness {
    pub engine: Engine,
    pub out: ManualHandle,
    pub sink: Arc<CollectingSink>,
    pub clock: ManualClock,
    pub latency: u64,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_opener(Arc::new(FsOpener))
    }

    pub fn with_opener(opener: Arc<dyn SourceOpener>) -> Self {
        let (output, out) = ManualOutput::new();
        let sink = Arc::new(CollectingSink::default());
        let clock = ManualClock::new(UtcMillis::new(1_000_000));
        let engine = Engine::start(
            EngineConfig {
                sample_rate: RATE,
                device: None,
                opener,
                sink: sink.clone(),
                clock: Arc::new(clock.clone()),
            },
            Box::new(output),
        )
        .expect("engine starts");
        // The first event of every engine is the stream it opened; tests of what follows start
        // from a clean sink (the event itself is covered in tests/engine.rs).
        assert!(
            matches!(sink.take().as_slice(), [EventKind::OutputOpened { .. }]),
            "an engine starts by reporting its output"
        );
        Self {
            engine,
            out,
            sink,
            clock,
            latency: 0,
        }
    }

    /// Wait (bounded spin, no clock) until the ring holds `frames`.
    pub fn wait_buffered(&self, frames: usize) {
        for _ in 0..30_000 {
            if self.out.buffered_frames().expect("stream open") >= frames {
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder did not deliver {frames} frames in time");
    }

    /// One callback of `frames` frames; returns the interleaved block and the real-audio frames.
    pub fn render_block(&mut self, frames: usize) -> (Vec<f32>, usize) {
        let mut block = vec![0.0f32; frames * 2];
        let real = self
            .out
            .render(&mut block, self.latency)
            .expect("stream open");
        self.engine.poll();
        (block, real)
    }

    /// Render exactly `frames` frames of real audio in `block`-sized callbacks, waiting for the
    /// decoder whenever the ring is short, so the result never contains underrun silence.
    pub fn render_exact(&mut self, frames: usize, block: usize) -> Vec<f32> {
        // The decoder needs callbacks to acknowledge the flush a command starts.
        self.settle();
        let mut rec = Vec::with_capacity(frames * 2);
        while rec.len() < frames * 2 {
            let want = block.min(frames - rec.len() / 2);
            self.wait_buffered(want);
            let (data, real) = self.render_block(want);
            assert_eq!(real, want, "callback was starved");
            rec.extend_from_slice(&data);
        }
        rec
    }

    /// Keep rendering (and discarding) until the decoder has carried out the last command, so
    /// everything rendered afterwards belongs to the new stream.
    ///
    /// The acknowledgement is an empty callback: it runs the flush handshake but cannot take any
    /// frame of the new stream, so nothing real is ever discarded here.
    pub fn settle(&mut self) {
        for _ in 0..30_000 {
            if !self.engine.command_pending() {
                return;
            }
            self.out.render(&mut [], self.latency).expect("stream open");
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("decoder never carried out the command");
    }

    /// Plays (480-frame callbacks) and polls in a bounded loop until `done` holds. Unlike
    /// `render_exact` it never waits for audio to arrive, so it cannot hang when the decoder has
    /// finished and the ring is empty; use it whenever the end state is an event, not a frame count.
    pub fn run_until(&mut self, what: &str, mut done: impl FnMut(&Harness) -> bool) {
        let mut block = vec![0.0f32; 960];
        for _ in 0..30_000 {
            self.engine.poll();
            if done(self) {
                return;
            }
            let real = self.out.render(&mut block, self.latency).unwrap_or(0);
            if real < 480 {
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
        }
        panic!("never happened: {what}");
    }

    /// True once `pred` matches an event emitted so far.
    pub fn saw(&self, pred: impl Fn(&EventKind) -> bool) -> bool {
        self.sink.kinds().iter().any(pred)
    }

    /// After the last real frame: callbacks return no audio and `QueueEnded` arrives.
    pub fn finish(&mut self) {
        for _ in 0..30_000 {
            let (_, real) = self.render_block(512);
            assert_eq!(real, 0, "audio beyond the expected length");
            if self.sink.kinds().contains(&EventKind::QueueEnded) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        panic!("QueueEnded never arrived");
    }
}

pub fn deinterleave(rec: &[f32]) -> (Vec<f32>, Vec<f32>) {
    (
        rec.chunks_exact(2).map(|f| f[0]).collect(),
        rec.chunks_exact(2).map(|f| f[1]).collect(),
    )
}

/// Largest absolute difference between `rec` (interleaved) and the reference starting at
/// `ref_start`.
pub fn max_err(rec: &[f32], ref_l: &[f32], ref_r: &[f32], ref_start: usize) -> f32 {
    let mut worst = 0.0f32;
    for (i, f) in rec.chunks_exact(2).enumerate() {
        worst = worst
            .max((f[0] - ref_l[ref_start + i]).abs())
            .max((f[1] - ref_r[ref_start + i]).abs());
    }
    worst
}

/// Which integer shift of the recording lines up best with the reference around `center`.
/// 0 means "no gap and no overlap".
pub fn best_lag(
    rec_l: &[f32],
    ref_l: &[f32],
    center: usize,
    half_window: usize,
    max_lag: i64,
) -> i64 {
    let (mut best, mut best_err) = (0i64, f64::MAX);
    for lag in -max_lag..=max_lag {
        let mut err = 0.0f64;
        for (i, &sample) in rec_l
            .iter()
            .enumerate()
            .take(center + half_window)
            .skip(center - half_window)
        {
            let r = (i as i64 + lag) as usize;
            let d = f64::from(sample) - f64::from(ref_l[r]);
            err += d * d;
        }
        if err < best_err {
            best_err = err;
            best = lag;
        }
    }
    best
}

/// Every `TrackStarted` is closed by exactly one `TrackLeft` for the same item before the next
/// `TrackStarted`; `allow_open` lets the last started track still be playing.
pub fn assert_paired(kinds: &[EventKind], allow_open: bool) {
    let mut open: Option<QueueItemId> = None;
    for kind in kinds {
        match kind {
            EventKind::TrackStarted { item, .. } => {
                assert!(
                    open.is_none(),
                    "{item:?} started while {open:?} is still open: {kinds:?}"
                );
                open = Some(*item);
            }
            EventKind::TrackLeft { item, .. } => {
                assert_eq!(
                    open,
                    Some(*item),
                    "TrackLeft without matching start: {kinds:?}"
                );
                open = None;
            }
            _ => {}
        }
    }
    assert!(
        allow_open || open.is_none(),
        "{open:?} was never closed: {kinds:?}"
    );
}

/// A WAV that reads fine up to `fail_after` bytes and then returns an IO error.
pub struct BrokenSource {
    inner: Cursor<Vec<u8>>,
    delivered: usize,
    fail_after: usize,
}

impl Read for BrokenSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.delivered >= self.fail_after {
            return Err(std::io::Error::other("disk went away"));
        }
        let n = self.inner.read(buf)?;
        self.delivered += n;
        Ok(n)
    }
}

impl Seek for BrokenSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for BrokenSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.inner.get_ref().len() as u64)
    }
}

/// Opens the named file normally, except `broken` which fails mid-stream.
#[derive(Debug)]
pub struct BreakingOpener {
    pub broken: std::path::PathBuf,
    pub fail_after: usize,
}

impl SourceOpener for BreakingOpener {
    fn open(&self, track: &Track) -> std::io::Result<Box<dyn MediaSource>> {
        let bytes = std::fs::read(&track.path)?;
        let fail_after = if track.path == self.broken {
            self.fail_after
        } else {
            usize::MAX
        };
        Ok(Box::new(BrokenSource {
            inner: Cursor::new(bytes),
            delivered: 0,
            fail_after,
        }))
    }
}

#[derive(Debug, Default)]
pub struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
    /// Set once the decoder thread is blocked in `read`.
    pub blocked: AtomicBool,
}

impl Gate {
    pub fn open(&self) {
        *self.open.lock().expect("gate") = true;
        self.changed.notify_all();
    }

    pub fn wait(&self) {
        let mut open = self.open.lock().expect("gate");
        self.blocked.store(true, Ordering::Release);
        while !*open {
            open = self.changed.wait(open).expect("gate");
        }
    }
}

/// An in-memory file that blocks every read once `allow` bytes were delivered, until the gate opens.
pub struct GatedSource {
    inner: Cursor<Vec<u8>>,
    gate: Arc<Gate>,
    /// Bytes handed out so far (seeks do not count) and the budget before reads block.
    delivered: u64,
    allow: u64,
}

impl Read for GatedSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.delivered >= self.allow {
            self.gate.wait();
        }
        let n = self.inner.read(buf)?;
        self.delivered += n as u64;
        Ok(n)
    }
}

impl Seek for GatedSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl MediaSource for GatedSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.inner.get_ref().len() as u64)
    }
}

#[derive(Debug)]
pub struct GatedOpener {
    pub gate: Arc<Gate>,
    pub allow: u64,
    /// Gate only this path; every other track reads freely.
    pub only: Option<std::path::PathBuf>,
}

impl SourceOpener for GatedOpener {
    fn open(&self, track: &Track) -> std::io::Result<Box<dyn MediaSource>> {
        let gated = self.only.as_ref().is_none_or(|p| *p == track.path);
        Ok(Box::new(GatedSource {
            inner: Cursor::new(std::fs::read(&track.path)?),
            gate: Arc::clone(&self.gate),
            delivered: 0,
            allow: if gated { self.allow } else { u64::MAX },
        }))
    }
}

// ---- a fake event-driven device: the WASAPI callback logic without WASAPI -------------------

use tinwave_playback::{
    AudioOutput, DeviceEvent, DeviceFollower, DeviceLost, FollowTarget, OpenRequest, OutputStream,
    PlaybackError, Pump, PumpStep, RenderDevice, Renderer, SampleFormat,
};

/// Records what the pump writes. Always has room for a full buffer, like a device that has just
/// played everything.
pub struct FakeDevice {
    buffer_frames: usize,
    pub recorded: Vec<u8>,
}

impl RenderDevice for FakeDevice {
    fn wait(&mut self, _timeout_ms: u32) -> Result<bool, DeviceLost> {
        Ok(true)
    }
    fn available_frames(&mut self) -> Result<usize, DeviceLost> {
        Ok(self.buffer_frames)
    }
    fn write(&mut self, _frames: usize, bytes: &[u8]) -> Result<(), DeviceLost> {
        self.recorded.extend_from_slice(bytes);
        Ok(())
    }
    fn is_alive(&mut self) -> bool {
        true
    }
    fn start(&mut self) -> Result<(), DeviceLost> {
        Ok(())
    }
    fn stop(&mut self) {}
}

struct Rig {
    pump: Pump,
    device: FakeDevice,
}

struct FakeShared {
    rig: Mutex<Option<Rig>>,
    lost: AtomicBool,
    follower: Mutex<Option<DeviceFollower>>,
    opens: Mutex<Vec<Option<String>>>,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An `AudioOutput` whose "audio callback" is the real `Pump` (render, convert, write) driven by
/// the test thread against a [`FakeDevice`].
pub struct FakeDeviceOutput {
    shared: Arc<FakeShared>,
    format: SampleFormat,
    buffer_frames: usize,
}

#[derive(Clone)]
pub struct FakeDeviceHandle {
    shared: Arc<FakeShared>,
}

struct FakeStream {
    shared: Arc<FakeShared>,
}

impl OutputStream for FakeStream {
    fn is_lost(&self) -> bool {
        self.shared.lost.load(Ordering::Acquire)
    }
}

impl Drop for FakeStream {
    fn drop(&mut self) {
        // As a real output: the callback is gone, the renderer is dropped by the caller's thread.
        let rig = locked(&self.shared.rig).take();
        drop(rig.map(|r| r.pump.into_renderer()));
    }
}

impl FakeDeviceOutput {
    pub fn new(format: SampleFormat, buffer_frames: usize) -> (Self, FakeDeviceHandle) {
        let shared = Arc::new(FakeShared {
            rig: Mutex::new(None),
            lost: AtomicBool::new(false),
            follower: Mutex::new(None),
            opens: Mutex::new(Vec::new()),
        });
        (
            Self {
                shared: Arc::clone(&shared),
                format,
                buffer_frames,
            },
            FakeDeviceHandle { shared },
        )
    }
}

impl AudioOutput for FakeDeviceOutput {
    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError> {
        let pump = Pump::new(renderer, self.format, self.buffer_frames, 0);
        let device = FakeDevice {
            buffer_frames: self.buffer_frames,
            recorded: Vec::with_capacity(1 << 20),
        };
        *locked(&self.shared.rig) = Some(Rig { pump, device });
        self.shared.lost.store(false, Ordering::Release);
        let (target, id) = match request.device {
            None => (FollowTarget::Default, "speakers".to_string()),
            Some(id) => (FollowTarget::Device(id.to_string()), id.to_string()),
        };
        *locked(&self.shared.follower) = Some(DeviceFollower::new(target, id));
        locked(&self.shared.opens).push(request.device.map(str::to_owned));
        Ok(Box::new(FakeStream {
            shared: Arc::clone(&self.shared),
        }))
    }
}

impl FakeDeviceHandle {
    /// One device wake-up.
    pub fn step(&self) -> Option<PumpStep> {
        let mut guard = locked(&self.shared.rig);
        let rig = guard.as_mut()?;
        let before = rig.device.recorded.len();
        let step = rig.pump.step(&mut rig.device, 0);
        // Keep only blocks that carry audio: flush acknowledgements and silence are not music.
        if !matches!(step, PumpStep::Fed { real, .. } if real > 0) {
            rig.device.recorded.truncate(before);
        }
        Some(step)
    }

    /// One wake-up whose bytes are discarded at once, so the recording never grows.
    pub fn step_discarding(&self) -> Option<PumpStep> {
        let mut guard = locked(&self.shared.rig);
        let rig = guard.as_mut()?;
        rig.device.recorded.clear();
        Some(rig.pump.step(&mut rig.device, 0))
    }

    /// Acknowledge a pending flush without consuming audio.
    pub fn service_flush(&self) {
        if let Some(rig) = locked(&self.shared.rig).as_mut() {
            rig.pump.renderer().render(&mut [], 0);
        }
    }

    pub fn buffered_frames(&self) -> usize {
        locked(&self.shared.rig)
            .as_mut()
            .map_or(0, |r| r.pump.renderer().buffered_frames())
    }

    /// Everything recorded so far, taken.
    pub fn take_recorded(&self) -> Vec<u8> {
        locked(&self.shared.rig)
            .as_mut()
            .map(|r| std::mem::take(&mut r.device.recorded))
            .unwrap_or_default()
    }

    /// A fake `IMMNotificationClient` callback.
    pub fn notify(&self, event: &DeviceEvent) {
        let reopen = locked(&self.shared.follower)
            .as_ref()
            .is_some_and(|f| f.needs_reopen(event));
        if reopen {
            self.shared.lost.store(true, Ordering::Release);
        }
    }

    pub fn opens(&self) -> Vec<Option<String>> {
        locked(&self.shared.opens).clone()
    }
}

/// Bytes the device received, read back as the f32 the renderer produced (for bit-exactness
/// checks): integers are scaled by their full-scale power of two.
pub fn decode_device_bytes(format: SampleFormat, bytes: &[u8]) -> Vec<f32> {
    match format {
        SampleFormat::F32 => bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        SampleFormat::I16 => bytes
            .chunks_exact(2)
            .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32_768.0)
            .collect(),
        SampleFormat::I24In32 => bytes
            .chunks_exact(4)
            .map(|c| (i32::from_le_bytes([c[0], c[1], c[2], c[3]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        SampleFormat::I32 => bytes
            .chunks_exact(4)
            .map(|c| {
                (f64::from(i32::from_le_bytes([c[0], c[1], c[2], c[3]])) / 2_147_483_648.0) as f32
            })
            .collect(),
    }
}
