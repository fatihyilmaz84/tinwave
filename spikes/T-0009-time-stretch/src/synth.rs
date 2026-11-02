//! Deterministic synthetic "music": drums from noise bursts, chord pad, sine melody. Stereo interleaved f32.
use std::f64::consts::PI;

pub const SR: u32 = 44_100;

pub struct Lcg(u64);
impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (((self.0 >> 33) as u32) as f32 / (1u32 << 31) as f32) * 2.0 - 1.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Kick,
    Snare,
    Hat,
}

#[derive(Clone, Copy, Debug)]
pub struct Onset {
    pub t: f64,
    pub kind: Kind,
}

pub struct Stems {
    pub drums: Vec<f32>,
    pub pad: Vec<f32>,
    pub melody: Vec<f32>,
    pub mix: Vec<f32>,
    pub onsets: Vec<Onset>,
    pub note_freqs: Vec<f64>,
    pub secs: f64,
}

const BPM_BEAT: f64 = 0.5; // 120 BPM
pub const NOTE_SECS: f64 = 0.5;
const PENTA: [f64; 7] = [587.33, 659.25, 783.99, 880.0, 987.77, 1174.66, 1318.51]; // D5 E5 G5 A5 B5 D6 E6

pub fn make(secs: f64) -> Stems {
    let n = (secs * SR as f64) as usize;
    let mut drums = vec![0.0f32; n * 2];
    let pad;
    let mut melody = vec![0.0f32; n * 2];
    let mut onsets = Vec::new();
    let mut rng = Lcg::new(12345);

    // Drums
    let mut t = 0.0;
    let mut step = 0usize;
    while t < secs - 0.3 {
        // step = 8th-note index (0.25 s). even step: beat. beat index = step/2
        let beat = step / 2;
        if step % 2 == 0 {
            if beat % 2 == 0 {
                onsets.push(Onset { t, kind: Kind::Kick });
                add_kick(&mut drums, t);
            } else {
                onsets.push(Onset { t, kind: Kind::Snare });
                add_snare(&mut drums, t, &mut rng);
            }
        } else {
            onsets.push(Onset { t, kind: Kind::Hat });
            add_hat(&mut drums, t, &mut rng);
        }
        step += 1;
        t = step as f64 * BPM_BEAT / 2.0;
    }

    pad = make_pad(n, 1.0);

    // Melody: 0.5 s sine notes (pure tones, known frequency), 15 ms attack/release
    let nn = (secs / NOTE_SECS) as usize;
    let mut note_freqs = Vec::new();
    let mut ph = 0.0f64;
    let pattern = [0usize, 2, 3, 4, 3, 2, 5, 4, 3, 1, 2, 0, 6, 5, 4, 2];
    for k in 0..nn {
        note_freqs.push(PENTA[pattern[k % pattern.len()]]);
    }
    for i in 0..n {
        let tt = i as f64 / SR as f64;
        let k = ((tt / NOTE_SECS) as usize).min(nn - 1);
        let tn = tt - k as f64 * NOTE_SECS;
        let env = (tn / 0.015).min(1.0) * ((NOTE_SECS - tn) / 0.015).clamp(0.0, 1.0);
        ph += 2.0 * PI * note_freqs[k] / SR as f64;
        let v = (ph.sin() * env * 0.25) as f32;
        melody[i * 2] = v;
        melody[i * 2 + 1] = v;
    }

    let mut mix = vec![0.0f32; n * 2];
    for i in 0..n * 2 {
        mix[i] = drums[i] * 0.6 + pad[i] * 1.0 + melody[i] * 0.8;
    }
    let peak = mix.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let g = 0.8 / peak;
    for v in mix.iter_mut() {
        *v *= g;
    }
    Stems { drums, pad, melody, mix, onsets, note_freqs, secs }
}

fn add(buf: &mut [f32], start: f64, mut f: impl FnMut(f64) -> (f32, f32), dur: f64) {
    let s = (start * SR as f64) as usize;
    let len = (dur * SR as f64) as usize;
    for j in 0..len {
        let i = s + j;
        if i * 2 + 1 >= buf.len() {
            break;
        }
        let (l, r) = f(j as f64 / SR as f64);
        buf[i * 2] += l;
        buf[i * 2 + 1] += r;
    }
}

fn add_kick(b: &mut [f32], t: f64) {
    let mut ph = 0.0;
    add(
        b,
        t,
        |x| {
            let f = 45.0 + 110.0 * (-x / 0.02).exp();
            ph += 2.0 * PI * f / SR as f64;
            let v = (ph.sin() * (-x / 0.04).exp()) as f32 * 0.9;
            (v, v)
        },
        0.3,
    );
}
fn add_snare(b: &mut [f32], t: f64, rng: &mut Lcg) {
    let mut lp = 0.0f32;
    let mut ph = 0.0;
    add(
        b,
        t,
        |x| {
            let n = rng.next_f();
            lp += 0.5 * (n - lp); // noise, softly low-passed
            ph += 2.0 * PI * 190.0 / SR as f64;
            let v = ((lp * 0.9 + 0.3 * ph.sin() as f32) * (-x / 0.05).exp() as f32) * 0.7;
            (v, v)
        },
        0.3,
    );
}
fn add_hat(b: &mut [f32], t: f64, rng: &mut Lcg) {
    let mut prev = 0.0f32;
    add(
        b,
        t,
        |x| {
            let n = rng.next_f();
            let hp = n - prev; // 1st-difference high-pass
            prev = n;
            let v = hp * (-x / 0.012).exp() as f32 * 0.35;
            (v * 0.9, v * 1.1)
        },
        0.15,
    );
}

/// Constant sine (stereo identical), `secs` long.
pub fn sine(freq: f64, secs: f64, amp: f32) -> Vec<f32> {
    let n = (secs * SR as f64) as usize;
    let mut v = vec![0.0f32; n * 2];
    for i in 0..n {
        let s = (2.0 * PI * freq * i as f64 / SR as f64).sin() as f32 * amp;
        v[i * 2] = s;
        v[i * 2 + 1] = s;
    }
    v
}

/// Chord pad, `n` frames. `rate` time-scales only the envelope / chord timing (the ideal output of a perfect
/// pitch-preserving time-stretch by `rate`): oscillator frequencies stay put, chord changes arrive `rate` times faster.
pub fn make_pad(n: usize, rate: f64) -> Vec<f32> {
    let mut pad = vec![0.0f32; n * 2];
    let chords: [[f64; 3]; 4] = [
        [220.0, 261.63, 329.63],
        [174.61, 220.0, 261.63],
        [261.63, 329.63, 392.0],
        [196.0, 246.94, 293.66],
    ];
    let bar = 2.0;
    for i in 0..n {
        let tt = i as f64 / SR as f64;
        let ts = tt * rate; // source-timeline position drives chord/envelope
        let ci = ((ts / bar) as usize) % 4;
        let tb = ts % bar;
        let env = (tb / 0.25).min(1.0) * ((bar - tb) / 0.15).clamp(0.0, 1.0);
        let (mut l, mut r) = (0.0f64, 0.0f64);
        for (k, &f) in chords[ci].iter().enumerate() {
            for h in 1..=8 {
                let a = 1.0 / h as f64;
                let fl = f * (1.0 - 0.0015) * h as f64;
                let fr = f * (1.0 + 0.0015) * h as f64;
                if fl < 8000.0 {
                    l += a * (2.0 * PI * fl * tt + k as f64).sin();
                    r += a * (2.0 * PI * fr * tt + 0.7 * k as f64).sin();
                }
            }
        }
        pad[i * 2] = (l * env * 0.08) as f32;
        pad[i * 2 + 1] = (r * env * 0.08) as f32;
    }
    pad
}
