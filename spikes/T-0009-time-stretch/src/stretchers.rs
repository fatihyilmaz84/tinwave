//! Uniform pull-style wrappers: `run` fills `out` with stretched audio, reading source frames from `src` at `*pos`.
//! `rate` = source seconds consumed per output second (1.25 = faster). Interleaved stereo f32 everywhere.
use rubberband_sys as rb;
use std::collections::VecDeque;
use std::num::{NonZeroU16, NonZeroU32};

pub const CH: usize = 2;

pub trait Stage: Send {
    fn name(&self) -> String;
    fn set_rate(&mut self, r: f64);
    /// Fill `out`; returns frames produced (< out.len()/2 only at end of source).
    fn run(&mut self, src: &[f32], pos: &mut usize, out: &mut [f32]) -> usize;
    fn reset(&mut self);
    /// Library-reported latency, where it reports one.
    fn reported_latency(&self) -> String;
    /// Pre-roll: feed history (frames before the current position) without affecting stream position, if supported.
    fn preroll(&mut self, _history: &[f32], _rate: f64) {}
}

// ---------------------------------------------------------------- Signalsmith
pub struct Sig {
    s: signalsmith_stretch::Stretch,
    rate: f64,
    frac: f64,
    label: &'static str,
}
impl Sig {
    pub fn custom(block: usize, interval: usize, label: &'static str) -> Self {
        Self { s: signalsmith_stretch::Stretch::new(2, block, interval), rate: 1.0, frac: 0.0, label }
    }
    pub fn new(sr: u32, cheaper: bool) -> Self {
        let s = if cheaper {
            signalsmith_stretch::Stretch::preset_cheaper(2, sr)
        } else {
            signalsmith_stretch::Stretch::preset_default(2, sr)
        };
        Self { s, rate: 1.0, frac: 0.0, label: if cheaper { "signalsmith-cheaper" } else { "signalsmith-default" } }
    }
}
impl Stage for Sig {
    fn name(&self) -> String {
        self.label.into()
    }
    fn set_rate(&mut self, r: f64) {
        self.rate = r;
    }
    fn run(&mut self, src: &[f32], pos: &mut usize, out: &mut [f32]) -> usize {
        let of = out.len() / CH;
        let ideal = of as f64 * self.rate + self.frac;
        let mut inn = ideal.floor() as usize;
        self.frac = ideal - inn as f64;
        let total = src.len() / CH;
        let mut produce = of;
        if *pos + inn > total {
            inn = total - *pos;
            produce = (inn as f64 / self.rate) as usize;
        }
        self.s.process(&src[*pos * CH..(*pos + inn) * CH], &mut out[..produce * CH]);
        *pos += inn;
        produce
    }
    fn reset(&mut self) {
        self.s.reset();
        self.frac = 0.0;
    }
    fn reported_latency(&self) -> String {
        format!("in {} + out {} frames", self.s.input_latency(), self.s.output_latency())
    }
    fn preroll(&mut self, history: &[f32], rate: f64) {
        self.s.seek(history, rate);
    }
}

// ---------------------------------------------------------------- Rubber Band (GPL-2.0-or-later / commercial)
pub struct Rubber {
    st: rb::RubberBandState,
    finer: bool,
    chunk: usize,
    pin: [Vec<f32>; 2],
    pout: [Vec<f32>; 2],
}
unsafe impl Send for Rubber {}
impl Rubber {
    pub fn new(sr: u32, finer: bool) -> Self {
        Self::new_with(sr, finer, 4096, 4096)
    }
    pub fn new_with(sr: u32, finer: bool, maxp: usize, chunk: usize) -> Self {
        let mut opt = rb::RubberBandOption_RubberBandOptionProcessRealTime | rb::RubberBandOption_RubberBandOptionThreadingNever;
        opt |= if finer { rb::RubberBandOption_RubberBandOptionEngineFiner } else { rb::RubberBandOption_RubberBandOptionEngineFaster };
        let st = unsafe { rb::rubberband_new(sr, 2, opt, 1.0, 1.0) };
        assert!(!st.is_null());
        unsafe { rb::rubberband_set_max_process_size(st, maxp as u32) };
        Self { st, finer, chunk, pin: [vec![0.0; 4096], vec![0.0; 4096]], pout: [vec![0.0; 8192], vec![0.0; 8192]] }
    }
}
impl Drop for Rubber {
    fn drop(&mut self) {
        unsafe { rb::rubberband_delete(self.st) }
    }
}
impl Stage for Rubber {
    fn name(&self) -> String {
        if self.finer {
            "rubberband-R3-finer".into()
        } else {
            "rubberband-R2-faster".into()
        }
    }
    fn set_rate(&mut self, r: f64) {
        unsafe { rb::rubberband_set_time_ratio(self.st, 1.0 / r) };
    }
    fn run(&mut self, src: &[f32], pos: &mut usize, out: &mut [f32]) -> usize {
        let of = out.len() / CH;
        let total = src.len() / CH;
        let mut got = 0;
        while got < of {
            let avail = unsafe { rb::rubberband_available(self.st) };
            if avail > 0 {
                let n = (avail as usize).min(of - got).min(8192);
                let ptrs = [self.pout[0].as_mut_ptr(), self.pout[1].as_mut_ptr()];
                let r = unsafe { rb::rubberband_retrieve(self.st, ptrs.as_ptr(), n as u32) } as usize;
                for i in 0..r {
                    out[(got + i) * CH] = self.pout[0][i];
                    out[(got + i) * CH + 1] = self.pout[1][i];
                }
                got += r;
                continue;
            }
            let req = unsafe { rb::rubberband_get_samples_required(self.st) } as usize;
            let n = req.max(1).min(self.chunk).min(total - *pos);
            if n == 0 {
                break;
            }
            for i in 0..n {
                self.pin[0][i] = src[(*pos + i) * CH];
                self.pin[1][i] = src[(*pos + i) * CH + 1];
            }
            let ptrs = [self.pin[0].as_ptr(), self.pin[1].as_ptr()];
            unsafe { rb::rubberband_process(self.st, ptrs.as_ptr(), n as u32, 0) };
            *pos += n;
        }
        got
    }
    fn reset(&mut self) {
        unsafe { rb::rubberband_reset(self.st) };
    }
    fn reported_latency(&self) -> String {
        unsafe {
            format!(
                "start_delay {} preferred_start_pad {} latency {} frames (engine R{})",
                rb::rubberband_get_start_delay(self.st),
                rb::rubberband_get_preferred_start_pad(self.st),
                rb::rubberband_get_latency(self.st),
                rb::rubberband_get_engine_version(self.st)
            )
        }
    }
}

// ---------------------------------------------------------------- WSOLA (pure Rust, MIT)
pub struct Wsola {
    ts: wsola::TimeStretch,
    fifo: VecDeque<f32>,
    scratch: Vec<f32>,
    hop_ms: f32,
    sr: u32,
}
impl Wsola {
    pub fn new(sr: u32, hop_ms: f32, search_ms: f32) -> Self {
        let ts = wsola::TimeStretch::with_config(
            NonZeroU32::new(sr).unwrap(),
            NonZeroU16::new(2).unwrap(),
            wsola::Config { hop_ms, search_ms },
        )
        .unwrap();
        let min = ts.min_output_buff_size();
        Self { ts, fifo: VecDeque::new(), scratch: vec![0.0; min * 4], hop_ms, sr }
    }
}
impl Stage for Wsola {
    fn name(&self) -> String {
        format!("wsola-hop{}ms", self.hop_ms)
    }
    fn set_rate(&mut self, r: f64) {
        self.ts.set_tempo(r as f32);
    }
    fn run(&mut self, src: &[f32], pos: &mut usize, out: &mut [f32]) -> usize {
        let of = out.len() / CH;
        let total = src.len() / CH;
        let want = of * CH;
        let chunk = 1024usize;
        while self.fifo.len() < want {
            match self.ts.pull(&mut self.scratch) {
                Ok(n) => self.fifo.extend(self.scratch[..n].iter().copied()),
                Err(_) => {
                    let n = chunk.min(total - *pos);
                    if n == 0 {
                        break;
                    }
                    self.ts.push(&src[*pos * CH..(*pos + n) * CH]);
                    *pos += n;
                }
            }
        }
        let n = want.min(self.fifo.len());
        for (i, v) in self.fifo.drain(..n).enumerate() {
            out[i] = v;
        }
        n / CH
    }
    fn reset(&mut self) {
        self.ts.reset();
        self.fifo.clear();
    }
    fn reported_latency(&self) -> String {
        let hop = (self.sr as f32 * self.hop_ms / 1000.0) as usize;
        format!("none reported (window ~{} frames)", hop * 2)
    }
}

pub const COUNT: usize = 6;

pub fn make(sr: u32, i: usize) -> Box<dyn Stage> {
    match i {
        0 => Box::new(Sig::new(sr, false)),
        1 => Box::new(Sig::new(sr, true)),
        2 => Box::new(Sig::custom(2048, 512, "signalsmith-b2048")),
        3 => Box::new(Rubber::new(sr, true)),
        4 => Box::new(Rubber::new(sr, false)),
        _ => Box::new(Wsola::new(sr, 15.0, 12.0)),
    }
}
