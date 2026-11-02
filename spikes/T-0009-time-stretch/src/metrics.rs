//! Objective artifact measures. All inputs interleaved stereo f32 at `SR`.
use crate::synth::{Kind, Onset, SR};
use rustfft::{num_complex::Complex, FftPlanner};

pub fn mono(x: &[f32]) -> Vec<f32> {
    x.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect()
}

pub fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len() as f64).sqrt()
}

fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos() as f32).collect()
}

/// Magnitude spectrum (n/2+1 bins) of a Hann-windowed frame.
pub fn spectrum(frame: &[f32], fft: &std::sync::Arc<dyn rustfft::Fft<f32>>, win: &[f32]) -> Vec<f32> {
    let n = frame.len();
    let mut buf: Vec<Complex<f32>> = (0..n).map(|i| Complex::new(frame[i] * win[i], 0.0)).collect();
    fft.process(&mut buf);
    buf[..n / 2 + 1].iter().map(|c| c.norm()).collect()
}

/// 1 ms-hop RMS envelope (2 ms window). Returns samples per ms hop = SR/1000.
pub fn envelope(x: &[f32]) -> Vec<f32> {
    let hop = (SR / 1000) as usize;
    let win = hop * 2;
    let mut e = Vec::new();
    let mut i = 0;
    while i + win <= x.len() {
        e.push(rms(&x[i..i + win]) as f32);
        i += hop;
    }
    e
}

/// Pitch error in cents against the nearest note of `notes`, per analysis frame of the melody stem.
/// Returns (median |cents|, p95 |cents|, max |cents|, fraction of frames within 5 cents).
pub fn pitch_error(out_mono: &[f32], notes: &[f64]) -> (f64, f64, f64, f64) {
    let n = 8192;
    let hop = 2048;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let win = hann(n);
    let mut errs = Vec::new();
    let mut i = 0;
    // skip first/last 0.4 s (latency fill and tail)
    let skip = (0.4 * SR as f64) as usize;
    let mut uniq: Vec<f64> = notes.to_vec();
    uniq.sort_by(|a, b| a.partial_cmp(b).unwrap());
    uniq.dedup();
    i += skip;
    while i + n + skip <= out_mono.len() {
        let sp = spectrum(&out_mono[i..i + n], &fft, &win);
        let (mut k, mut m) = (0, 0.0f32);
        for (j, &v) in sp.iter().enumerate().skip(10) {
            if v > m {
                m = v;
                k = j;
            }
        }
        if m > 0.5 && k > 1 && k + 1 < sp.len() {
            let (a, b, c) = ((sp[k - 1] + 1e-12).ln() as f64, (sp[k] + 1e-12).ln() as f64, (sp[k + 1] + 1e-12).ln() as f64);
            let d = 0.5 * (a - c) / (a - 2.0 * b + c);
            let f = (k as f64 + d) * SR as f64 / n as f64;
            let nearest = uniq.iter().cloned().min_by(|x, y| (x / f).ln().abs().partial_cmp(&(y / f).ln().abs()).unwrap()).unwrap();
            errs.push((1200.0 * (f / nearest).log2()).abs());
        }
        i += hop;
    }
    if errs.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    }
    errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = errs[errs.len() / 2];
    let p95 = errs[(errs.len() as f64 * 0.95) as usize];
    let within = errs.iter().filter(|&&e| e < 5.0).count() as f64 / errs.len() as f64;
    (med, p95, *errs.last().unwrap(), within)
}

pub struct Transient {
    pub lag_ms: f64,
    pub rise_ms: f64,
    pub rise_ms_hat: f64,
    pub pre_echo_db: f64,
    pub peak_ratio_db: f64,
    pub count_ratio: f64,
}

fn peak_in(e: &[f32], a: isize, b: isize) -> (usize, f32) {
    let (mut p, mut m) = (0usize, 0.0f32);
    for i in a.max(0)..b.min(e.len() as isize) {
        if e[i as usize] > m {
            m = e[i as usize];
            p = i as usize;
        }
    }
    (p, m)
}

/// Drum transient quality. `rate` is the stretch rate (source/output). `ref_peak` normalises the
/// peak (use the unprocessed drum stem's mean peak so the ratio is in dB relative to the source).
pub fn transients(out_mono: &[f32], onsets: &[Onset], rate: f64) -> Transient {
    let e = envelope(out_mono);
    // global lag: maximise summed envelope at the kick positions
    let kicks: Vec<&Onset> = onsets.iter().filter(|o| o.kind == Kind::Kick).collect();
    let (mut best_lag, mut best) = (0isize, -1.0f64);
    for lag in -300isize..700 {
        let mut s = 0.0;
        for o in &kicks {
            let c = (o.t / rate * 1000.0) as isize + lag;
            let (_, m) = peak_in(&e, c - 4, c + 5);
            s += m as f64;
        }
        if s > best {
            best = s;
            best_lag = lag;
        }
    }
    let mut rises = Vec::new();
    let mut rises_hat = Vec::new();
    let mut pres = Vec::new();
    let mut peaks = Vec::new();
    let mut found = 0usize;
    for o in onsets {
        let c = (o.t / rate * 1000.0) as isize + best_lag;
        if c < 80 || (c + 80) as usize >= e.len() {
            continue;
        }
        let (p, m) = peak_in(&e, c - 25, c + 25);
        if m < 1e-4 {
            continue;
        }
        found += 1;
        // rise time: walk back from peak to 90% / 10%
        let mut t10 = p;
        let mut t90 = p;
        let mut i = p;
        while i > 0 && e[i] > 0.1 * m {
            if e[i] > 0.9 * m {
                t90 = i;
            }
            i -= 1;
            t10 = i;
        }
        let rise = (t90 as f64 - t10 as f64).max(0.0);
        rises.push(rise);
        if o.kind == Kind::Hat {
            rises_hat.push(rise);
        }
        // pre-echo: RMS of envelope 40..12 ms before the 10% point, relative to peak
        let pe_a = t10 as isize - 40;
        let pe_b = t10 as isize - 12;
        if pe_a > 0 {
            let seg = &e[pe_a as usize..pe_b as usize];
            let r = (seg.iter().map(|&v| (v * v) as f64).sum::<f64>() / seg.len() as f64).sqrt();
            pres.push(20.0 * ((r / m as f64) + 1e-9).log10());
        }
        peaks.push(m as f64);
    }
    let mean = |v: &Vec<f64>| if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 };
    // count peaks in output (separated >= 60 ms, above 15% of mean onset peak) vs expected
    let thr = 0.15 * mean(&peaks) as f32;
    let mut count = 0;
    let mut i = 0;
    while i < e.len() {
        if e[i] > thr {
            let (p, _) = peak_in(&e, i as isize, i as isize + 60);
            count += 1;
            i = p + 60;
        } else {
            i += 1;
        }
    }
    Transient {
        lag_ms: best_lag as f64,
        rise_ms: mean(&rises),
        rise_ms_hat: mean(&rises_hat),
        pre_echo_db: mean(&pres),
        peak_ratio_db: 20.0 * mean(&peaks).log10(),
        count_ratio: count as f64 / onsets.len() as f64,
    }
}

/// Mean normalised spectral flux (positive log-magnitude change between consecutive frames), 2048 FFT hop 512.
pub fn spectral_flux(x_mono: &[f32]) -> f64 {
    let n = 2048;
    let hop = 512;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let win = hann(n);
    let mut prev: Option<Vec<f32>> = None;
    let (mut acc, mut cnt) = (0.0f64, 0usize);
    let skip = (0.5 * SR as f64) as usize;
    let mut i = skip;
    while i + n + skip <= x_mono.len() {
        let sp = spectrum(&x_mono[i..i + n], &fft, &win);
        if let Some(p) = &prev {
            let mut f = 0.0f64;
            let mut tot = 0.0f64;
            for k in 1..sp.len() / 2 {
                let d = sp[k] - p[k];
                if d > 0.0 {
                    f += d as f64;
                }
                tot += sp[k] as f64;
            }
            acc += f / (tot + 1e-9);
            cnt += 1;
        }
        prev = Some(sp);
        i += hop;
    }
    acc / cnt.max(1) as f64
}

/// Tonal purity of a steady sine: dB of (power within +-3 bins of the peak) / (power elsewhere), 16384 FFT.
pub fn sine_purity_db(x_mono: &[f32]) -> f64 {
    let n = 16384;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let win = hann(n);
    let start = x_mono.len() / 2 - n / 2;
    let sp = spectrum(&x_mono[start..start + n], &fft, &win);
    let (mut k, mut m) = (0, 0.0);
    for (j, &v) in sp.iter().enumerate().skip(4) {
        if v > m {
            m = v;
            k = j;
        }
    }
    let (mut s, mut o) = (0.0f64, 0.0f64);
    for (j, &v) in sp.iter().enumerate().skip(2) {
        let p = (v as f64) * (v as f64);
        if (j as isize - k as isize).abs() <= 3 {
            s += p;
        } else {
            o += p;
        }
    }
    10.0 * (s / (o + 1e-18)).log10()
}

/// Amplitude-modulation depth of a steady tone: (max-min)/mean of a 5 ms RMS envelope in the middle 60%.
pub fn tremolo_depth(x_mono: &[f32]) -> f64 {
    let w = (SR as usize * 5) / 1000;
    let a = x_mono.len() / 5;
    let b = x_mono.len() * 4 / 5;
    let mut mn = f64::MAX;
    let mut mx = 0.0f64;
    let mut sum = 0.0;
    let mut c = 0;
    let mut i = a;
    while i + w < b {
        let r = rms(&x_mono[i..i + w]);
        mn = mn.min(r);
        mx = mx.max(r);
        sum += r;
        c += 1;
        i += w;
    }
    (mx - mn) / (sum / c as f64)
}

/// L/R correlation coefficient.
pub fn lr_corr(x: &[f32]) -> f64 {
    let (mut sl, mut sr, mut sll, mut srr, mut slr) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    let n = (x.len() / 2) as f64;
    for c in x.chunks_exact(2) {
        let (l, r) = (c[0] as f64, c[1] as f64);
        sl += l;
        sr += r;
        sll += l * l;
        srr += r * r;
        slr += l * r;
    }
    let cov = slr / n - (sl / n) * (sr / n);
    let vl = sll / n - (sl / n).powi(2);
    let vr = srr / n - (sr / n).powi(2);
    cov / (vl.sqrt() * vr.sqrt() + 1e-18)
}

/// Log-spectral distance (dB) between `out` and the ideal stretched reference, after finding the output lag by
/// envelope cross-correlation. Frames within 200 ms of a chord change are skipped (the stretcher legitimately
/// smears those). Bins 100 Hz - 8 kHz where the reference is within 50 dB of its frame peak.
/// Returns (lsd_db, lag_ms).
pub fn lsd_vs_ideal(out_mono: &[f32], ideal_mono: &[f32], rate: f64) -> (f64, f64) {
    let hop = (SR / 100) as usize; // 10 ms envelope
    let env = |x: &[f32]| -> Vec<f64> { x.chunks(hop).map(|c| rms(c)).collect() };
    let eo = env(out_mono);
    let ei = env(ideal_mono);
    let mut best = (f64::MIN, 0isize);
    for lag in -30isize..60 {
        let (mut s, mut c) = (0.0, 0);
        for i in 40..ei.len().saturating_sub(100) {
            let j = i as isize + lag;
            if j >= 0 && (j as usize) < eo.len() {
                s += ei[i] * eo[j as usize];
                c += 1;
            }
        }
        let s = s / c.max(1) as f64;
        if s > best.0 {
            best = (s, lag);
        }
    }
    let lag_samples = best.1 * hop as isize;
    let n = 4096;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let win = hann(n);
    let (mut acc, mut cnt) = (0.0f64, 0usize);
    let bar_out = 2.0 / rate * SR as f64;
    let mut i = (0.6 * SR as f64) as usize;
    let k0 = (100.0 * n as f64 / SR as f64) as usize;
    let k1 = (8000.0 * n as f64 / SR as f64) as usize;
    while i + n < ideal_mono.len().saturating_sub((0.6 * SR as f64) as usize) {
        let j = i as isize + lag_samples;
        let centre = i as f64 + n as f64 / 2.0;
        let phase = (centre % bar_out).min(bar_out - centre % bar_out);
        if j >= 0 && (j as usize) + n <= out_mono.len() && phase > 0.25 * SR as f64 {
            let so = spectrum(&out_mono[j as usize..j as usize + n], &fft, &win);
            let si = spectrum(&ideal_mono[i..i + n], &fft, &win);
            let pk = si[k0..k1].iter().cloned().fold(0.0f32, f32::max);
            let (mut d, mut m) = (0.0f64, 0);
            for k in k0..k1 {
                if si[k] > pk * 0.003 {
                    let a = 20.0 * ((so[k] + 1e-6) as f64).log10();
                    let b = 20.0 * ((si[k] + 1e-6) as f64).log10();
                    d += (a - b) * (a - b);
                    m += 1;
                }
            }
            if m > 0 {
                acc += (d / m as f64).sqrt();
                cnt += 1;
            }
        }
        i += n / 2;
    }
    (acc / cnt.max(1) as f64, best.1 as f64 * 10.0)
}
