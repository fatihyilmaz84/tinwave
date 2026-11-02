mod engine_sim;
mod metrics;
mod stretchers;
mod synth;
mod sys;

use stretchers::*;
use synth::SR;

#[global_allocator]
static A: sys::Counting = sys::Counting;

pub fn stretch_all(st: &mut dyn Stage, src: &[f32], rate: f64) -> Vec<f32> {
    st.set_rate(rate);
    let mut out = Vec::with_capacity((src.len() as f64 / rate) as usize + 8192);
    let mut blk = vec![0.0f32; 1024 * CH];
    let mut pos = 0usize;
    loop {
        let n = st.run(src, &mut pos, &mut blk);
        out.extend_from_slice(&blk[..n * CH]);
        if n < 1024 {
            break;
        }
    }
    out
}

fn write_wav(path: &str, x: &[f32]) {
    let spec = hound::WavSpec { channels: 2, sample_rate: SR, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &v in x {
        w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

fn names() -> Vec<usize> {
    (0..stretchers::COUNT).collect()
}

fn fresh(i: usize) -> Box<dyn Stage> {
    stretchers::make(SR, i)
}

fn cmd_quality(write: bool) {
    let st = synth::make(32.0);
    if write {
        std::fs::create_dir_all("out").unwrap();
        write_wav("out/source_mix.wav", &st.mix);
        write_wav("out/source_drums.wav", &st.drums);
    }
    let tone = synth::sine(1000.0, 6.0, 0.5);
    let ref_flux_pad = metrics::spectral_flux(&metrics::mono(&st.pad));
    let ref_flux_mix = metrics::spectral_flux(&metrics::mono(&st.mix));
    let ref_tr = metrics::transients(&metrics::mono(&st.drums), &st.onsets, 1.0);
    println!(
        "SOURCE: drums rise(all) {:.2} ms rise(hat) {:.2} ms pre-echo {:.1} dB peak {:.1} dB | flux pad {:.4} mix {:.4} | pad L/R corr {:.3}",
        ref_tr.rise_ms,
        ref_tr.rise_ms_hat,
        ref_tr.pre_echo_db,
        ref_tr.peak_ratio_db,
        ref_flux_pad,
        ref_flux_mix,
        metrics::lr_corr(&st.pad)
    );
    println!("rate | option | pitch err cents med/p95/max | <5c | drums: lag ms, rise ms (hat), pre-echo dB, peak dB(rel src) | pad LSD dB | pad flux x | mix flux x | 1k sine purity dB, tremolo % | pad L/R corr | level dB");
    let rates = [0.75, 1.25, 1.5];
    for &rate in &rates {
        for i in 0..names().len() {
            let nm = fresh(i).name();
            let mel = stretch_all(&mut *fresh(i), &st.melody, rate);
            let (med, p95, max, within) = metrics::pitch_error(&metrics::mono(&mel), &st.note_freqs);
            let dr = stretch_all(&mut *fresh(i), &st.drums, rate);
            let tr = metrics::transients(&metrics::mono(&dr), &st.onsets, rate);
            let pad = stretch_all(&mut *fresh(i), &st.pad, rate);
            let ideal = synth::make_pad(pad.len() / 2, rate);
            let (lsd, _lag) = metrics::lsd_vs_ideal(&metrics::mono(&pad), &metrics::mono(&ideal), rate);
            let pflux = metrics::spectral_flux(&metrics::mono(&pad)) / ref_flux_pad;
            let mix = stretch_all(&mut *fresh(i), &st.mix, rate);
            let mflux = metrics::spectral_flux(&metrics::mono(&mix)) / ref_flux_mix;
            let tn = stretch_all(&mut *fresh(i), &tone, rate);
            let tm = metrics::mono(&tn);
            let purity = metrics::sine_purity_db(&tm);
            let trem = metrics::tremolo_depth(&tm) * 100.0;
            let corr = metrics::lr_corr(&pad[pad.len() / 4..pad.len() * 3 / 4]);
            let lvl = 20.0 * (metrics::rms(&mix[mix.len() / 4..mix.len() * 3 / 4]) / metrics::rms(&st.mix[st.mix.len() / 4..st.mix.len() * 3 / 4])).log10();
            println!(
                "{rate:.2} | {nm:22} | {med:5.2}/{p95:5.2}/{max:6.2} | {:3.0}% | {:5.0}, {:5.2} ({:5.2}), {:6.1}, {:5.1} | {lsd:5.2} | {pflux:4.2} | {mflux:4.2} | {purity:5.1}, {trem:5.2} | {corr:5.3} | {lvl:+5.2}",
                within * 100.0,
                tr.lag_ms,
                tr.rise_ms,
                tr.rise_ms_hat,
                tr.pre_echo_db,
                tr.peak_ratio_db - ref_tr.peak_ratio_db,
            );
            if write {
                write_wav(&format!("out/{nm}_{rate:.2}x_mix.wav"), &mix);
            }
        }
    }
    // sanity: the reference at rate 1.0 (identity) for the metrics themselves
    let id = &st.melody;
    let (med, p95, max, within) = metrics::pitch_error(&metrics::mono(id), &st.note_freqs);
    println!("metric floor (unprocessed melody): pitch err {med:.2}/{p95:.2}/{max:.2} cents, {:.0}% < 5c", within * 100.0);
}

fn cmd_bench() {
    sys::high_priority();
    sys::pin_to_core(5);
    let st = synth::make(60.0);
    let secs_src = st.secs;
    println!("# CPU on ONE pinned core (core 5), HIGH priority, 44.1 kHz stereo, {secs_src:.0} s synthetic music mix, 1024-frame output blocks, best of 5");
    println!("option | rate | wall s per 60 s of output audio-equivalent | x realtime | % of one core when playing realtime | heap allocs during run (Rust + C++ operator new) | private MB delta (create+run)");
    for i in 0..names().len() {
        let nm = fresh(i).name();
        for &rate in &[0.75, 1.0, 1.25, 1.5] {
            let mut best = f64::MAX;
            let mut allocs = 0;
            let mut mem_delta = 0.0;
            for _rep in 0..5 {
                let (_, p0) = sys::mem();
                let mut s = fresh(i);
                s.set_rate(rate);
                let mut out = vec![0.0f32; 1024 * CH];
                let mut pos = 0usize;
                // warm-up: ~2 s so internal buffer allocation is excluded
                for _ in 0..(44100 * 2 / 1024) {
                    s.run(&st.mix, &mut pos, &mut out);
                }
                let a0 = sys::ALLOCS.load(std::sync::atomic::Ordering::Relaxed) + sys::cpp_allocs();
                let t0 = std::time::Instant::now();
                let mut produced = 0usize;
                loop {
                    let n = s.run(&st.mix, &mut pos, &mut out);
                    produced += n;
                    if n < 1024 {
                        break;
                    }
                }
                let wall = t0.elapsed().as_secs_f64();
                let a1 = sys::ALLOCS.load(std::sync::atomic::Ordering::Relaxed) + sys::cpp_allocs();
                let (_, p1) = sys::mem();
                let out_secs = produced as f64 / SR as f64;
                let per60 = wall / out_secs * 60.0;
                if per60 < best {
                    best = per60;
                    allocs = a1 - a0;
                    mem_delta = (p1 as f64 - p0 as f64) / 1e6;
                }
            }
            println!("{nm:22} | {rate:.2} | {best:6.3} | {:7.0}x | {:6.3}% | {allocs} | {mem_delta:.1}", 60.0 / best, best / 60.0 * 100.0);
        }
    }
}

fn cmd_mem() {
    let st = synth::make(8.0);
    println!("# memory per stretcher instance (44.1 kHz stereo): private-bytes delta over 8 instances, each run for 6 s of audio at 1.25x");
    for i in 0..names().len() {
        let nm = fresh(i).name();
        let (_, p0) = sys::mem();
        let c0 = sys::cpp_allocs();
        let mut v: Vec<Box<dyn Stage>> = Vec::new();
        for _ in 0..8 {
            let mut s = fresh(i);
            s.set_rate(1.25);
            let mut pos = 0;
            let mut out = vec![0.0f32; 1024 * CH];
            for _ in 0..(6 * 44100 / 1024) {
                s.run(&st.mix, &mut pos, &mut out);
            }
            v.push(s);
        }
        let (_, p1) = sys::mem();
        println!("{nm:22} | {:.2} MB per instance | {} C++ allocs during construct+warm-up for 8 instances", (p1 as f64 - p0 as f64) / 8.0 / 1e6, sys::cpp_allocs() - c0);
    }
}

fn cmd_latency() {
    println!("# algorithmic latency: 3 ms click at source frame S, constant rate; lag = (output click position) - S/rate, in ms, at 44.1 kHz");
    println!("option | library-reported | rate | lag ms (argmax) | click energy spread (ms, 5%..95% energy)");
    let s_frame = 4 * SR as usize;
    let mut src = vec![0.0f32; 10 * SR as usize * 2];
    let mut rng = synth::Lcg::new(7);
    let len = (0.003 * SR as f64) as usize;
    for j in 0..len {
        let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * j as f64 / len as f64).cos();
        let v = rng.next_f() * w as f32 * 0.8;
        src[(s_frame + j) * 2] = v;
        src[(s_frame + j) * 2 + 1] = v;
    }
    for i in 0..names().len() {
        let nm = fresh(i).name();
        let rep = fresh(i).reported_latency();
        for &rate in &[0.75, 1.0, 1.25, 1.5] {
            let mut s = fresh(i);
            let out = stretch_all(&mut *s, &src, rate);
            let m = metrics::mono(&out);
            let exp = (s_frame as f64 + len as f64 / 2.0) / rate;
            let lo = (exp as isize - 20000).max(0) as usize;
            let hi = ((exp as isize + 20000) as usize).min(m.len());
            let (mut p, mut mx) = (lo, 0.0f32);
            for k in lo..hi {
                if m[k].abs() > mx {
                    mx = m[k].abs();
                    p = k;
                }
            }
            let tot: f64 = m[lo..hi].iter().map(|&v| (v as f64).powi(2)).sum();
            let mut cum = 0.0;
            let (mut a, mut b) = (lo, hi);
            let mut got_a = false;
            for k in lo..hi {
                cum += (m[k] as f64).powi(2);
                if !got_a && cum > 0.05 * tot {
                    a = k;
                    got_a = true;
                }
                if cum > 0.95 * tot {
                    b = k;
                    break;
                }
            }
            println!("{nm:22} | {rep} | {rate:.2} | {:7.1} | {:6.1}", (p as f64 - exp) * 1000.0 / SR as f64, (b - a) as f64 * 1000.0 / SR as f64);
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("quality") => cmd_quality(a.get(2).map(|s| s == "wav").unwrap_or(false)),
        Some("bench") => cmd_bench(),
        Some("mem") => cmd_mem(),
        Some("latency") => cmd_latency(),
        Some("r2test") => {
            let tone = synth::sine(1000.0, 6.0, 0.5);
            for finer in [false, true] {
                for &(maxp, chunk) in &[(4096usize, 4096usize), (1024, 1024), (512, 512), (256, 256)] {
                    let mut r = Rubber::new_with(SR, finer, maxp, chunk);
                    let out = stretch_all(&mut r, &tone, 1.25);
                    let tm = metrics::mono(&out);
                    println!("finer={finer} maxproc={maxp} chunk={chunk}: purity {:.1} dB tremolo {:.2}%", metrics::sine_purity_db(&tm), metrics::tremolo_depth(&tm) * 100.0);
                }
            }
        }
        Some("engine") => engine_sim::run(&a[2..]),
        _ => eprintln!("usage: quality [wav] | bench | latency | engine <test>"),
    }
}
