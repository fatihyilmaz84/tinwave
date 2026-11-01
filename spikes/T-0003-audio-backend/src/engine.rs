//! Backend-independent playback core: decoder thread -> SPSC ring -> `Renderer` (runs in the audio callback).
//!
//! Real-time rules enforced in `Renderer::render`: no allocation, no locks (try_lock only for the optional
//! test recorder), no syscalls other than a monotonic clock read. All control is via atomics.
use crate::reader::TrackReader;
use anyhow::Result;
use rtrb::{Consumer, Producer, RingBuffer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const CH: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutSpec {
    pub rate: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub start_frame: u64,
    pub track: usize,
    pub offset: u64,
}

pub struct Shared {
    pub epoch: Instant,
    pub consumed: AtomicU64,
    pub latency_frames: AtomicU64,
    pub volume: AtomicU32,
    pub paused: AtomicBool,
    pub flush_req: AtomicU64,
    pub flush_ack: AtomicU64,
    pub underruns: AtomicU64,
    pub eos: AtomicBool,
    pub ended: AtomicBool,
    pub cmd_ns: AtomicU64,
    pub first_audio_ns: AtomicU64,
    pub callbacks: AtomicU64,
    pub cb_max_ns: AtomicU64,
    pub cb_total_ns: AtomicU64,
    pub marks: Mutex<Vec<Mark>>,
    pub log: Mutex<Vec<String>>,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            epoch: Instant::now(),
            consumed: AtomicU64::new(0),
            latency_frames: AtomicU64::new(0),
            volume: AtomicU32::new(1.0f32.to_bits()),
            paused: AtomicBool::new(false),
            flush_req: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            eos: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            cmd_ns: AtomicU64::new(0),
            first_audio_ns: AtomicU64::new(0),
            callbacks: AtomicU64::new(0),
            cb_max_ns: AtomicU64::new(0),
            cb_total_ns: AtomicU64::new(0),
            marks: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
        })
    }
    pub fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }
    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }
}

/// Lives inside the audio callback.
pub struct Renderer {
    cons: Consumer<f32>,
    sh: Arc<Shared>,
    gain: f32,
    rec: Option<Arc<Mutex<Vec<f32>>>>,
}

impl Renderer {
    pub fn new(cons: Consumer<f32>, sh: Arc<Shared>, rec: Option<Arc<Mutex<Vec<f32>>>>) -> Self {
        let gain = f32::from_bits(sh.volume.load(Relaxed));
        Self { cons, sh, gain, rec }
    }

    /// Fill `out` (interleaved stereo f32). `latency_frames` = frames between this write and the speaker.
    pub fn render(&mut self, out: &mut [f32], latency_frames: u64) {
        let t0 = Instant::now();
        let sh = &*self.sh;
        sh.latency_frames.store(latency_frames, Relaxed);

        // Seek / reconfigure handshake: drain the ring while the decoder is parked.
        let req = sh.flush_req.load(Acquire);
        if req != sh.flush_ack.load(Relaxed) {
            let mut sink = [0.0f32; 512];
            while !self.cons.is_empty() {
                let _ = self.cons.pop_partial_slice(&mut sink);
            }
            sh.consumed.store(0, Release);
            sh.flush_ack.store(req, Release);
            out.fill(0.0);
            self.finish(t0, out, 0);
            return;
        }
        if sh.paused.load(Relaxed) {
            out.fill(0.0);
            self.finish(t0, out, 0);
            return;
        }

        let eos = sh.eos.load(Acquire); // read BEFORE the ring so "eos && empty" is conclusive
        let (popped, rest) = self.cons.pop_partial_slice(out);
        let n = popped.len();
        rest.fill(0.0);
        if n < out.len() {
            if eos {
                if n == 0 {
                    sh.ended.store(true, Release);
                }
            } else {
                sh.underruns.fetch_add(1, Relaxed);
            }
        }
        // Software volume, linearly ramped across the block (no zipper noise, no allocation).
        let target = f32::from_bits(sh.volume.load(Relaxed));
        if n > 0 {
            let frames = n / CH;
            let start = self.gain;
            if (target - start).abs() < 1e-6 {
                if (start - 1.0).abs() > 1e-6 {
                    for s in &mut out[..n] {
                        *s *= start;
                    }
                }
            } else {
                let step = (target - start) / frames as f32;
                let mut g = start;
                for f in 0..frames {
                    g += step;
                    out[f * CH] *= g;
                    out[f * CH + 1] *= g;
                }
            }
            self.gain = target;
            sh.consumed.fetch_add(frames as u64, Release);
            if sh.first_audio_ns.load(Relaxed) == 0 {
                sh.first_audio_ns.store(sh.now_ns().max(1), Relaxed);
            }
        }
        self.finish(t0, out, n);
    }

    #[inline]
    fn finish(&mut self, t0: Instant, out: &[f32], _n: usize) {
        if let Some(rec) = &self.rec {
            if let Ok(mut v) = rec.try_lock() {
                if v.len() + out.len() <= v.capacity() {
                    v.extend_from_slice(out); // capacity reserved up front: never reallocates
                }
            }
        }
        let ns = t0.elapsed().as_nanos() as u64;
        self.sh.callbacks.fetch_add(1, Relaxed);
        self.sh.cb_total_ns.fetch_add(ns, Relaxed);
        self.sh.cb_max_ns.fetch_max(ns, Relaxed);
    }
}

pub enum Cmd {
    Seek { track: usize, frame: u64 },
    Reconfigure { prod: Producer<f32>, track: usize, frame: u64 },
    Stop,
}

struct Dec {
    paths: Vec<PathBuf>,
    sh: Arc<Shared>,
    prod: Producer<f32>,
    rx: Receiver<Cmd>,
    cur: usize,
    reader: Option<TrackReader>,
    pending: Vec<f32>,
    pend_off: usize,
    written: u64,
    gapless: bool,
    rate: u32,
    trims: Vec<Option<(u64, u64)>>,
}

impl Dec {
    fn open_current(&mut self, frame: u64) -> bool {
        while self.cur < self.paths.len() {
            match TrackReader::open(&self.paths[self.cur], self.gapless) {
                Ok(mut r) => {
                    if r.rate != self.rate {
                        self.sh.note(format!("track {} rate {} != output {}: skipped (spike has no resampler)", self.cur, r.rate, self.rate));
                        self.cur += 1;
                        continue;
                    }
                    if let Some((d, v)) = self.trims[self.cur] {
                        r.set_gapless_override(d, v);
                    }
                    if frame > 0 {
                        if let Err(e) = r.seek(frame) {
                            self.sh.note(format!("seek failed: {e:#}"));
                        }
                    }
                    self.sh.marks.lock().unwrap().push(Mark { start_frame: self.written, track: self.cur, offset: frame });
                    self.reader = Some(r);
                    return true;
                }
                Err(e) => {
                    self.sh.note(format!("open {}: {e:#}", self.paths[self.cur].display()));
                    self.cur += 1;
                }
            }
        }
        false
    }

    fn reset_stream_state(&mut self) {
        self.pending.clear();
        self.pend_off = 0;
        self.written = 0;
        self.sh.marks.lock().unwrap().clear();
        self.sh.eos.store(false, Release);
        self.sh.ended.store(false, Release);
    }

    /// Returns false when the thread should exit.
    fn handle(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Stop => return false,
            Cmd::Seek { track, frame } => {
                let req = self.sh.flush_req.fetch_add(1, AcqRel) + 1;
                let deadline = Instant::now() + Duration::from_millis(500);
                while self.sh.flush_ack.load(Acquire) != req && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(1));
                }
                if self.sh.flush_ack.load(Acquire) != req {
                    self.sh.note("flush ack timeout (stream not running?)".into());
                }
                self.reset_stream_state();
                self.cur = track;
                self.reader = None;
                self.open_current(frame);
            }
            Cmd::Reconfigure { prod, track, frame } => {
                self.prod = prod;
                // Old stream is gone; nothing can race. Align the handshake counters.
                let req = self.sh.flush_req.load(Acquire);
                self.sh.flush_ack.store(req, Release);
                self.sh.consumed.store(0, Release);
                self.reset_stream_state();
                self.cur = track;
                self.reader = None;
                self.open_current(frame);
            }
        }
        true
    }

    fn run(mut self, start_track: usize, start_frame: u64) {
        self.cur = start_track;
        self.open_current(start_frame);
        loop {
            while let Ok(cmd) = self.rx.try_recv() {
                if !self.handle(cmd) {
                    return;
                }
            }
            if self.pend_off >= self.pending.len() {
                self.pending.clear();
                self.pend_off = 0;
                if self.reader.is_none() && !self.sh.eos.load(Relaxed) {
                    if self.cur < self.paths.len() && !self.open_current(0) {
                        // fallthrough to eos
                    }
                }
                match self.reader.as_mut() {
                    Some(r) => match r.next_chunk(&mut self.pending) {
                        Ok(true) => {}
                        Ok(false) => {
                            self.reader = None;
                            self.cur += 1;
                            if self.cur >= self.paths.len() {
                                self.sh.eos.store(true, Release);
                            }
                            continue;
                        }
                        Err(e) => {
                            self.sh.note(format!("decode error: {e:#}"));
                            self.reader = None;
                            self.cur += 1;
                            if self.cur >= self.paths.len() {
                                self.sh.eos.store(true, Release);
                            }
                            continue;
                        }
                    },
                    None => {
                        if self.cur >= self.paths.len() {
                            self.sh.eos.store(true, Release);
                        }
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                }
            }
            let (pushed, _) = self.prod.push_partial_slice(&self.pending[self.pend_off..]);
            let n = pushed.len();
            self.pend_off += n;
            self.written += (n / CH) as u64;
            if n == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

pub trait Backend {
    fn name(&self) -> String;
    fn spec(&self) -> OutSpec;
    /// Set when the device was lost / invalidated / default changed (stream must be rebuilt).
    fn lost(&self) -> bool;
    fn mode(&self) -> String {
        "shared".into()
    }
}

pub type OpenFn = dyn Fn(Option<&str>, u32, Renderer) -> Result<Box<dyn Backend>>;

pub struct Position {
    pub track: usize,
    pub frames: u64,
    pub ms: f64,
    pub consumed: u64,
    pub underruns: u64,
    pub ended: bool,
}

pub struct Engine {
    pub sh: Arc<Shared>,
    tx: Sender<Cmd>,
    th: Option<JoinHandle<()>>,
    pub backend: Option<Box<dyn Backend>>,
    pub rate: u32,
    pub rec: Option<Arc<Mutex<Vec<f32>>>>,
    open: Box<OpenFn>,
    pub device: Option<String>,
}

impl Engine {
    /// `rate` = output stream rate (files must match; the OS converts to the device mix rate).
    pub fn start(
        paths: Vec<PathBuf>,
        trims: Vec<Option<(u64, u64)>>,
        rate: u32,
        device: Option<String>,
        gapless: bool,
        record_secs: Option<f64>,
        open: Box<OpenFn>,
    ) -> Result<Self> {
        let sh = Shared::new();
        sh.cmd_ns.store(sh.now_ns(), Relaxed);
        let (prod, cons) = RingBuffer::<f32>::new(rate as usize * CH); // 1 s
        let (tx, rx) = channel();
        let rec = record_secs.map(|s| Arc::new(Mutex::new(Vec::with_capacity((s * rate as f64) as usize * CH))));
        let dec = Dec {
            paths,
            sh: sh.clone(),
            prod,
            rx,
            cur: 0,
            reader: None,
            pending: Vec::new(),
            pend_off: 0,
            written: 0,
            gapless,
            rate,
            trims,
        };
        let th = std::thread::Builder::new().name("decoder".into()).spawn(move || dec.run(0, 0))?;
        let renderer = Renderer::new(cons, sh.clone(), rec.clone());
        let backend = open(device.as_deref(), rate, renderer)?;
        Ok(Self { sh, tx, th: Some(th), backend: Some(backend), rate, rec, open, device })
    }

    pub fn seek(&self, track: usize, frame: u64) {
        self.sh.first_audio_ns.store(0, Relaxed);
        self.sh.cmd_ns.store(self.sh.now_ns(), Relaxed);
        let _ = self.tx.send(Cmd::Seek { track, frame });
    }

    pub fn set_volume(&self, v: f32) {
        self.sh.volume.store(v.to_bits(), Relaxed);
    }

    pub fn set_paused(&self, p: bool) {
        self.sh.paused.store(p, Relaxed);
    }

    pub fn position(&self) -> Position {
        let consumed = self.sh.consumed.load(Acquire);
        let lat = self.sh.latency_frames.load(Relaxed);
        let played = consumed.saturating_sub(lat);
        let marks = self.sh.marks.lock().unwrap();
        let mut track = 0;
        let mut frames = 0;
        for m in marks.iter() {
            if m.start_frame <= played {
                track = m.track;
                frames = played - m.start_frame + m.offset;
            }
        }
        drop(marks);
        Position {
            track,
            frames,
            ms: frames as f64 * 1000.0 / self.rate as f64,
            consumed,
            underruns: self.sh.underruns.load(Relaxed),
            ended: self.sh.ended.load(Acquire),
        }
    }

    /// Move to another output device (None = system default) and resume where we were.
    /// Returns the time until the new stream delivered its first real audio callback.
    pub fn switch_device(&mut self, device: Option<String>) -> Result<Duration> {
        let t0 = Instant::now();
        let pos = self.position();
        self.backend = None; // stops the old stream
        let (prod, cons) = RingBuffer::<f32>::new(self.rate as usize * CH);
        self.sh.first_audio_ns.store(0, Relaxed);
        let _ = self.tx.send(Cmd::Reconfigure { prod, track: pos.track, frame: pos.frames });
        let renderer = Renderer::new(cons, self.sh.clone(), self.rec.clone());
        let b = (self.open)(device.as_deref(), self.rate, renderer)?;
        self.backend = Some(b);
        self.device = device;
        while self.sh.first_audio_ns.load(Relaxed) == 0 && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(t0.elapsed())
    }

    pub fn stop(mut self) -> Vec<String> {
        self.backend = None;
        let _ = self.tx.send(Cmd::Stop);
        if let Some(t) = self.th.take() {
            let _ = t.join();
        }
        self.sh.log.lock().unwrap().clone()
    }
}
