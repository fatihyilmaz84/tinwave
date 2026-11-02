//! Integration sketch + measurements: time-stretch lives in the DECODER thread of the T-0003 design
//! (decoder thread -> SPSC ring -> RT renderer). The RT callback only pops frames, applies fades and does
//! O(1) rate bookkeeping. The "device" is a simulated 10 ms callback thread (no sound card needed; T-0003
//! already measured real WASAPI/cpal behaviour), so every number here is about the engine, not about drivers.
//!
//! A rate change is a "flush to the audible source position and restart" (the T-0003 seek handshake):
//!   control: rate.store(r); flush_req += 1
//!   renderer: fade out <= 5 ms of the queued old audio, drop the rest of the ring, publish how far the source
//!             advanced (src_micro), ack
//!   decoder : audible position A = base + src_micro; reopen at A; if the new rate != 1.0 reset the stretcher,
//!             feed from A and discard the first `lag_out` output frames (fast, ~1 ms CPU) so the first delivered
//!             frame IS the audio at A; renderer fades the new stream in over 5 ms.
use crate::stretchers::*;
use crate::synth::{self, SR};
use crate::sys;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BLK: usize = 512; // decoder production block (frames)
const FADE: usize = 220; // 5 ms at 44.1 kHz
const CB: usize = 441; // simulated device period: 10 ms
const RING_FRAMES: usize = SR as usize; // 1 s

#[derive(Clone, Copy)]
struct Seg {
    out_start: u64,
    rate: f32,
}

#[derive(Clone, Copy, Default)]
pub struct BlockRec {
    pub t_ns: u64,
    pub eff_rate: f32,
    pub real_frames: u32,
    pub epoch: u32,
    pub src_epoch_micro: u64, // source advance since last flush at block START (micro-frames)
}

pub struct Shared {
    epoch0: Instant,
    rate: AtomicU64,
    flush_req: AtomicU64,
    flush_ack: AtomicU64,
    ack_src_micro: AtomicU64,
    pub eff_rate: AtomicU32,
    pub tot_wall: AtomicU64,
    pub tot_src_micro: AtomicU64,
    pub underruns: AtomicU64,
    pub eos: AtomicBool,
    stop: AtomicBool,
    pub restarts: Mutex<Vec<(u32, f64, f64)>>, // (epoch, base source frame, new rate)
}
impl Shared {
    fn now_ns(&self) -> u64 {
        self.epoch0.elapsed().as_nanos() as u64
    }
}

// ------------------------------------------------------------------------------------------ renderer (RT)
pub struct Renderer {
    cons: Consumer<f32>,
    segs: Consumer<Seg>,
    sh: Arc<Shared>,
    cur_rate: f32,
    out_pos: u64,
    src_epoch: u64,
    fade_in_left: usize,
    epoch: u32,
    pub rec: Vec<f32>,
    pub blocks: Vec<BlockRec>,
    pub max_cb_ns: u64,
}

impl Renderer {
    fn account(&mut self, frames: usize) -> u64 {
        let mut done = 0usize;
        let mut src = 0u64;
        while done < frames {
            while let Ok(s) = self.segs.peek() {
                if s.out_start <= self.out_pos + done as u64 {
                    self.cur_rate = s.rate;
                    let _ = self.segs.pop();
                } else {
                    break;
                }
            }
            let limit = match self.segs.peek() {
                Ok(s) => (s.out_start - (self.out_pos + done as u64)) as usize,
                Err(_) => usize::MAX,
            };
            let take = (frames - done).min(limit);
            src += (take as f64 * self.cur_rate as f64 * 1e6) as u64;
            done += take;
        }
        self.out_pos += frames as u64;
        src
    }

    /// Fill `out` (interleaved stereo, CB frames). No allocation, no locks, no syscalls except the clock read.
    pub fn render(&mut self, out: &mut [f32]) {
        sys::IN_RT.with(|c| c.set(true));
        let t0 = Instant::now();
        self.render_inner(out);
        let ns = t0.elapsed().as_nanos() as u64;
        self.max_cb_ns = self.max_cb_ns.max(ns);
        sys::IN_RT.with(|c| c.set(false));
    }

    fn render_inner(&mut self, out: &mut [f32]) {
        let sh = self.sh.clone();
        let t_ns = sh.now_ns();
        let src_at_start = self.src_epoch;
        let epoch_at_start = self.epoch;
        let req = sh.flush_req.load(Acquire);
        let real;
        if req != sh.flush_ack.load(Relaxed) {
            // fade out up to FADE frames of the queued old audio, then drop everything
            let avail = self.cons.slots() / 2;
            let n = avail.min(FADE).min(out.len() / 2);
            let (a, b) = out.split_at_mut(n * 2);
            let _ = self.cons.pop_partial_slice(a);
            for i in 0..n {
                let g = 1.0 - (i as f32 + 1.0) / FADE as f32;
                a[i * 2] *= g;
                a[i * 2 + 1] *= g;
            }
            b.fill(0.0);
            let src_adv = self.account(n);
            real = n;
            let mut sink = [0.0f32; 512];
            while !self.cons.is_empty() {
                let _ = self.cons.pop_partial_slice(&mut sink);
            }
            while self.segs.pop().is_ok() {}
            self.src_epoch += src_adv;
            sh.ack_src_micro.store(self.src_epoch, Release);
            sh.tot_src_micro.fetch_add(src_adv, Relaxed);
            sh.tot_wall.fetch_add(real as u64, Relaxed);
            self.src_epoch = 0;
            self.out_pos = 0;
            self.fade_in_left = FADE;
            self.epoch = req as u32;
            let eff = if real > 0 { (src_adv as f64 / real as f64 / 1e6) as f32 } else { 0.0 };
            sh.eff_rate.store(eff.to_bits(), Relaxed);
            sh.flush_ack.store(req, Release);
        } else {
            let eos = sh.eos.load(Acquire);
            let (popped, rest) = self.cons.pop_partial_slice(out);
            let n = popped.len() / 2;
            rest.fill(0.0);
            if n * 2 < out.len() && !eos {
                sh.underruns.fetch_add(1, Relaxed);
            }
            for i in 0..n {
                if self.fade_in_left > 0 {
                    let g = 1.0 - self.fade_in_left as f32 / FADE as f32;
                    out[i * 2] *= g;
                    out[i * 2 + 1] *= g;
                    self.fade_in_left -= 1;
                }
            }
            let mut src_adv = 0;
            if n > 0 {
                src_adv = self.account(n);
                self.src_epoch += src_adv;
                sh.tot_src_micro.fetch_add(src_adv, Relaxed);
                sh.tot_wall.fetch_add(n as u64, Relaxed);
            }
            real = n;
            let eff = if n > 0 { (src_adv as f64 / n as f64 / 1e6) as f32 } else { 0.0 };
            sh.eff_rate.store(eff.to_bits(), Relaxed);
        }
        if self.rec.len() + out.len() <= self.rec.capacity() {
            self.rec.extend_from_slice(out); // capacity reserved up front: never reallocates
        }
        if self.blocks.len() < self.blocks.capacity() {
            let eff = f32::from_bits(sh.eff_rate.load(Relaxed));
            self.blocks.push(BlockRec { t_ns, eff_rate: eff, real_frames: real as u32, epoch: epoch_at_start, src_epoch_micro: src_at_start });
        }
    }
}

// ------------------------------------------------------------------------------------------ decoder thread
struct Dec {
    src: Arc<Vec<f32>>,
    st: Box<dyn Stage>,
    sh: Arc<Shared>,
    prod: Producer<f32>,
    segp: Producer<Seg>,
    cursor: usize,
    written: u64,
    base_micro: f64,
    cur_rate: f64,
    last_seg: f32,
    handled: u64,
    fill_target: usize, // frames
    flush_mode: bool,
    lag_out: fn(f64) -> usize,
}

fn lag_signalsmith_default(rate: f64) -> usize {
    // measured (results/latency.txt): output click lag = out_latency + in_latency / rate
    (2646.0 + 2646.0 / rate).ceil() as usize
}

impl Dec {
    fn restart(&mut self, a_micro: f64, rate: f64, epoch: u32) {
        let a = (a_micro / 1e6).round().max(0.0) as usize;
        self.cursor = a;
        self.base_micro = a as f64 * 1e6;
        self.written = 0;
        self.last_seg = f32::NAN;
        self.cur_rate = rate;
        if rate != 1.0 {
            self.st.reset();
            self.st.set_rate(rate);
            // fast prime: discard the stretcher's output latency so the first delivered frame is the audio at A
            let mut left = (self.lag_out)(rate);
            let mut scratch = [0.0f32; BLK * 2];
            while left > 0 {
                let n = left.min(BLK);
                let got = self.st.run(&self.src, &mut self.cursor, &mut scratch[..n * 2]);
                if got == 0 {
                    break;
                }
                left -= got;
            }
        }
        self.sh.restarts.lock().unwrap().push((epoch, self.base_micro / 1e6, rate));
    }

    fn produce(&mut self) -> bool {
        let free = self.prod.slots();
        let occupied = RING_FRAMES * 2 - free;
        if free < BLK * 2 || occupied / 2 >= self.fill_target {
            return false;
        }
        let total = self.src.len() / 2;
        let mut buf = [0.0f32; BLK * 2];
        let n;
        if self.cur_rate == 1.0 {
            n = BLK.min(total.saturating_sub(self.cursor));
            buf[..n * 2].copy_from_slice(&self.src[self.cursor * 2..(self.cursor + n) * 2]);
            self.cursor += n;
        } else {
            n = self.st.run(&self.src, &mut self.cursor, &mut buf);
        }
        if n == 0 {
            self.sh.eos.store(true, Release);
            return false;
        }
        if self.last_seg != self.cur_rate as f32 {
            while self.segp.push(Seg { out_start: self.written, rate: self.cur_rate as f32 }).is_err() {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.last_seg = self.cur_rate as f32;
        }
        let (pushed, _) = self.prod.push_partial_slice(&buf[..n * 2]);
        debug_assert_eq!(pushed.len(), n * 2);
        self.written += n as u64;
        true
    }

    fn run(mut self, start: f64, rate: f64) {
        self.restart(start * 1e6, rate, 0);
        while !self.sh.stop.load(Relaxed) {
            let req = self.sh.flush_req.load(Acquire);
            if req != self.handled {
                let deadline = Instant::now() + Duration::from_millis(500);
                while self.sh.flush_ack.load(Acquire) != req && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_micros(200));
                }
                let new_rate = f64::from_bits(self.sh.rate.load(Acquire));
                let a_micro = self.base_micro + self.sh.ack_src_micro.load(Acquire) as f64;
                self.restart(a_micro, new_rate, req as u32);
                self.handled = req;
                continue;
            }
            if !self.flush_mode {
                let new_rate = f64::from_bits(self.sh.rate.load(Acquire));
                if new_rate != self.cur_rate && new_rate != 1.0 && self.cur_rate != 1.0 {
                    self.st.set_rate(new_rate);
                    self.cur_rate = new_rate;
                }
            }
            if !self.produce() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

// ------------------------------------------------------------------------------------------ engine + fake device
pub struct Engine {
    pub sh: Arc<Shared>,
    dev: Option<std::thread::JoinHandle<Renderer>>,
    dec: Option<std::thread::JoinHandle<()>>,
    flush_mode: bool,
}

pub struct Opts {
    pub stage: usize,
    pub flush_mode: bool,
    pub fill_ms: usize,
    pub run_ms: u64,
    pub start_rate: f64,
}

impl Engine {
    pub fn start(src: Arc<Vec<f32>>, o: &Opts) -> Engine {
        let sh = Arc::new(Shared {
            epoch0: Instant::now(),
            rate: AtomicU64::new(o.start_rate.to_bits()),
            flush_req: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            ack_src_micro: AtomicU64::new(0),
            eff_rate: AtomicU32::new(0),
            tot_wall: AtomicU64::new(0),
            tot_src_micro: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            eos: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            restarts: Mutex::new(Vec::new()),
        });
        let (prod, cons) = RingBuffer::<f32>::new(RING_FRAMES * 2);
        let (segp, segc) = RingBuffer::<Seg>::new(64);
        let dec = Dec {
            src,
            st: make(SR, o.stage),
            sh: sh.clone(),
            prod,
            segp,
            cursor: 0,
            written: 0,
            base_micro: 0.0,
            cur_rate: 1.0,
            last_seg: f32::NAN,
            handled: 0,
            fill_target: if o.fill_ms == 0 { RING_FRAMES } else { o.fill_ms * SR as usize / 1000 },
            flush_mode: o.flush_mode,
            lag_out: lag_signalsmith_default,
        };
        let start_rate = o.start_rate;
        let dh = std::thread::Builder::new().name("decoder".into()).spawn(move || dec.run(0.0, start_rate)).unwrap();
        // fake device: 10 ms callbacks on a high-resolution timer
        let nblocks = (o.run_ms / 10) as usize + 50;
        let mut rend = Renderer {
            cons,
            segs: segc,
            sh: sh.clone(),
            cur_rate: 1.0,
            out_pos: 0,
            src_epoch: 0,
            fade_in_left: 0,
            epoch: 0,
            rec: Vec::with_capacity(nblocks * CB * 2),
            blocks: Vec::with_capacity(nblocks),
            max_cb_ns: 0,
        };
        let shd = sh.clone();
        let run_ms = o.run_ms;
        let dev = std::thread::Builder::new()
            .name("fake-device".into())
            .spawn(move || {
                unsafe { sys::timeBeginPeriod(1) };
                let mut buf = vec![0.0f32; CB * 2];
                let t0 = Instant::now();
                let mut k = 0u64;
                while !shd.stop.load(Relaxed) && t0.elapsed() < Duration::from_millis(run_ms) {
                    k += 1;
                    let deadline = t0 + Duration::from_millis(10 * k);
                    while Instant::now() + Duration::from_micros(1500) < deadline {
                        std::thread::sleep(Duration::from_micros(500));
                    }
                    while Instant::now() < deadline {
                        std::hint::spin_loop();
                    }
                    rend.render(&mut buf);
                }
                rend
            })
            .unwrap();
        Engine { sh, dev: Some(dev), dec: Some(dh), flush_mode: o.flush_mode }
    }

    pub fn set_rate(&mut self, r: f64) {
        self.sh.rate.store(r.to_bits(), Release);
        if self.flush_mode {
            self.sh.flush_req.fetch_add(1, AcqRel);
        }
    }
    pub fn now_ns(&self) -> u64 {
        self.sh.now_ns()
    }
    pub fn finish(mut self) -> (Renderer, Arc<Shared>) {
        let r = self.dev.take().unwrap().join().unwrap();
        self.sh.stop.store(true, Relaxed);
        self.dec.take().unwrap().join().unwrap();
        (r, self.sh.clone())
    }
}

fn sleep_until(e: &Engine, t_ns: u64) {
    while e.now_ns() < t_ns {
        std::thread::sleep(Duration::from_millis(1));
    }
}

// ------------------------------------------------------------------------------------------ tests
fn click_train(secs: f64, period: f64) -> Vec<f32> {
    // 440 Hz quiet bed + 2 ms noise-burst click every `period` seconds (click k is at source frame k*period*SR)
    let n = (secs * SR as f64) as usize;
    let mut v = synth::sine(440.0, secs, 0.1);
    let mut rng = synth::Lcg::new(99);
    let len = (0.002 * SR as f64) as usize;
    let mut k = 1usize;
    loop {
        let s = (k as f64 * period * SR as f64) as usize;
        if s + len >= n {
            break;
        }
        for j in 0..len {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * j as f64 / len as f64).cos() as f32;
            let x = rng.next_f() * w * 0.9;
            v[(s + j) * 2] += x;
            v[(s + j) * 2 + 1] += x;
        }
        k += 1;
    }
    v
}

pub fn run(args: &[String]) {
    let which = args.first().map(|s| s.as_str()).unwrap_or("all");
    let stage = 0usize; // signalsmith-default
    if which == "alloc" || which == "all" {
        t_alloc(stage);
    }
    if which == "lat" || which == "all" {
        t_latency(stage);
    }
    if which == "session" || which == "all" {
        t_session(stage);
    }
    if which == "splice" || which == "all" {
        t_splice(stage);
    }
}

fn t_alloc(stage: usize) {
    println!("## T1 RT safety: allocations inside Renderer::render while rates change (counting global allocator, IN_RT flag)");
    let src = Arc::new(synth::make(30.0).mix);
    let mut e = Engine::start(src, &Opts { stage, flush_mode: true, fill_ms: 0, run_ms: 9000, start_rate: 1.0 });
    let a0 = sys::RT_ALLOCS.load(Relaxed);
    for (t, r) in [(1000u64, 1.25), (2500, 0.75), (4000, 1.5), (5500, 1.0), (7000, 1.25)] {
        sleep_until(&e, t * 1_000_000);
        e.set_rate(r);
    }
    let (rend, sh) = e.finish();
    let a1 = sys::RT_ALLOCS.load(Relaxed);
    println!(
        "callbacks {} | allocs/frees inside render: {} | max callback {:.1} us | underruns {} | rate changes {}",
        rend.blocks.len(),
        a1 - a0,
        rend.max_cb_ns as f64 / 1000.0,
        sh.underruns.load(Relaxed),
        5
    );
}

fn t_splice(stage: usize) {
    println!("## T4 splice quality at a rate change (440 Hz sine, 0.5 amplitude): dip depth/duration and largest sample step");
    let sine = Arc::new(synth::sine(440.0, 30.0, 0.5));
    for (label, flush, from, to) in [
        ("flush 1.00 -> 1.25 (bypass->stretch)", true, 1.0, 1.25),
        ("flush 1.25 -> 0.75 (stretch->stretch)", true, 1.25, 0.75),
        ("flush 1.50 -> 1.00 (stretch->bypass)", true, 1.5, 1.0),
        ("no-flush 1.25 -> 0.75 (continuous set_rate, fill 150 ms)", false, 1.25, 0.75),
    ] {
        let mut e = Engine::start(sine.clone(), &Opts { stage, flush_mode: flush, fill_ms: if flush { 0 } else { 150 }, run_ms: 6000, start_rate: from });
        sleep_until(&e, 3_000_000_000);
        let t_call = e.now_ns();
        e.set_rate(to);
        let (rend, _) = e.finish();
        let m: Vec<f32> = rend.rec.chunks_exact(2).map(|c| c[0]).collect();
        let bi = rend.blocks.iter().position(|b| b.t_ns >= t_call).unwrap_or(0);
        let c0 = bi * CB;
        let w = (SR as usize) / 200; // 5 ms
        let steady = crate::metrics::rms(&m[c0 - 20 * w..c0 - 5 * w]);
        let (mut min_rel, mut dip_frames) = (1e9f64, 0usize);
        let mut i = c0;
        while i + w < c0 + SR as usize / 2 {
            let r = crate::metrics::rms(&m[i..i + w]) / steady;
            min_rel = min_rel.min(r);
            if r < 0.7 {
                dip_frames += w;
            }
            i += w;
        }
        let mut maxstep = 0.0f32;
        for k in c0..(c0 + SR as usize / 2).min(m.len() - 1) {
            maxstep = maxstep.max((m[k + 1] - m[k]).abs());
        }
        println!(
            "{label}: deepest 5 ms RMS {:.1}% of steady, time below 70% = {:.0} ms, max sample step {:.3} (steady sine {:.3})",
            min_rel * 100.0,
            dip_frames as f64 * 1000.0 / SR as f64,
            maxstep,
            0.5 * 2.0 * std::f64::consts::PI * 440.0 / SR as f64
        );
    }
}

fn t_latency(stage: usize) {
    println!("## T2 rate-change latency (engine level): wall time from set_rate() to the first rendered block that carries the new effective rate");
    println!("(device period 10 ms, ring 1 s; 'block at new rate' = first block with real frames whose effective rate is closest to the new rate)");
    let src = Arc::new(synth::make(40.0).mix);
    for (label, flush, fill, from, to) in [
        ("flush, full ring", true, 0usize, 1.0, 1.25),
        ("flush, full ring", true, 0, 1.25, 0.75),
        ("flush, full ring", true, 0, 0.75, 1.0),
        ("no flush, ring fill target 150 ms", false, 150, 1.25, 0.75),
        ("no flush, ring fill target 300 ms", false, 300, 1.25, 0.75),
        ("no flush, full ring (1 s)", false, 0, 1.25, 0.75),
    ] {
        let mut lats = Vec::new();
        for rep in 0..5 {
            let mut e = Engine::start(src.clone(), &Opts { stage, flush_mode: flush, fill_ms: fill, run_ms: 5500, start_rate: from });
            sleep_until(&e, (2_000 + 37 * rep as u64) * 1_000_000); // vary phase vs the 10 ms device clock
            let t_call = e.now_ns();
            e.set_rate(to);
            let (rend, _) = e.finish();
            let mid = (from + to) / 2.0;
            // first block after the call, with real audio, on the NEW side of the midpoint
            for b in rend.blocks.iter().filter(|b| b.t_ns >= t_call) {
                let on_new = if to > from { b.eff_rate as f64 > mid } else { (b.eff_rate as f64) < mid };
                if b.real_frames as usize >= CB && on_new {
                    lats.push((b.t_ns - t_call) as f64 / 1e6);
                    break;
                }
            }
        }
        lats.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let s = if lats.is_empty() { "n/a".to_string() } else { format!("min {:.0} / median {:.0} / max {:.0} ms ({} runs)", lats[0], lats[lats.len() / 2], lats[lats.len() - 1], lats.len()) };
        println!("{from:.2} -> {to:.2} | {label:36} | {s}");
    }
}

fn t_session(stage: usize) {
    println!("## T3 RateSession + position accounting: click train every 250 ms in the source, rate schedule applied by wall clock");
    let src = Arc::new(click_train(60.0, 0.25));
    for flush in [true, false] {
        let sched: [(u64, f64); 5] = [(0, 1.0), (3000, 1.25), (6000, 0.75), (8000, 1.5), (10000, 1.0)];
        let total_ms = 12_000u64;
        let mut e = Engine::start(src.clone(), &Opts { stage, flush_mode: flush, fill_ms: if flush { 0 } else { 150 }, run_ms: total_ms, start_rate: if flush { 1.0 } else { 1.25 } });
        let mut applied: Vec<(u64, f64)> = vec![(0, if flush { 1.0 } else { 1.25 })];
        for &(t, r) in sched.iter().skip(1) {
            sleep_until(&e, t * 1_000_000);
            if !flush && r == 1.0 {
                continue; // no-flush mode cannot do bypass<->stretch
            }
            e.set_rate(r);
            applied.push((t, r));
        }
        let (rend, sh) = e.finish();
        let wall = sh.tot_wall.load(Relaxed) as f64;
        let src_adv = sh.tot_src_micro.load(Relaxed) as f64 / 1e6;
        let twr = src_adv / wall;
        let mut exp_num = 0.0;
        for (i, &(t, r)) in applied.iter().enumerate() {
            let end = applied.get(i + 1).map(|x| x.0).unwrap_or(total_ms);
            exp_num += r * (end - t) as f64;
        }
        let exp = exp_num / total_ms as f64;
        let restarts = sh.restarts.lock().unwrap().clone();
        let m: Vec<f32> = rend.rec.chunks_exact(2).map(|c| c[0]).collect();
        // remove the 440 Hz bed: second difference emphasises the broadband click
        let mut hp = vec![0.0f32; m.len()];
        for i in 2..m.len() {
            hp[i] = m[i] - 2.0 * m[i - 1] + m[i - 2];
        }
        let env = crate::metrics::envelope(&hp);
        let mut errs: Vec<f64> = Vec::new();
        let mut i = 100usize;
        let thr = env.iter().cloned().fold(0.0f32, f32::max) * 0.25;
        while i + 100 < env.len() {
            if env[i] > thr && env[i] >= env[i - 1] && env[i] >= env[i + 1] {
                let f = i * (SR as usize / 1000) + SR as usize / 1000; // device frame of the click peak
                let bi = f / CB;
                if let Some(b) = rend.blocks.get(bi) {
                    if b.real_frames as usize >= CB && b.eff_rate > 0.0 {
                        if let Some(&(_, base, _)) = restarts.iter().find(|r| r.0 == b.epoch) {
                            let src_pos = base + b.src_epoch_micro as f64 / 1e6 + (f - bi * CB) as f64 * b.eff_rate as f64;
                            let period = 0.25 * SR as f64;
                            let k = (src_pos / period).round();
                            errs.push((src_pos - k * period) * 1000.0 / SR as f64);
                        }
                    }
                }
                i += 60;
            } else {
                i += 1;
            }
        }
        let n = errs.len();
        let mean_abs = errs.iter().map(|e| e.abs()).sum::<f64>() / n.max(1) as f64;
        let mean = errs.iter().sum::<f64>() / n.max(1) as f64;
        let maxabs = errs.iter().fold(0.0f64, |a, e| a.max(e.abs()));
        println!(
            "{}: time-weighted rate (engine) {:.4} vs wall-clock schedule {:.4} (diff {:+.4}) | wall frames {:.0}, source frames {:.0} | position error vs ground truth over {} clicks: mean {:+.1} ms, mean|e| {:.1} ms, max|e| {:.1} ms | underruns {}",
            if flush { "flush mode" } else { "no-flush mode (stretch<->stretch only, 1.0 step skipped)" },
            twr,
            exp,
            twr - exp,
            wall,
            src_adv,
            n,
            mean,
            mean_abs,
            maxabs,
            sh.underruns.load(Relaxed)
        );
    }
}
