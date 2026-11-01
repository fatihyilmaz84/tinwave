//! Measurement commands built on the engine: gapless boundary, seek/volume, device switch, CPU/RSS.
use crate::backends::*;
use crate::engine::{Backend as _, Engine, OpenFn, Renderer};
use crate::probe::{best_lag, compare, load_ref};
use crate::sys;
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::*;
use std::time::{Duration, Instant};

pub fn opener(kind: &str) -> Result<Box<OpenFn>> {
    Ok(match kind {
        "cpal" => Box::new(|d: Option<&str>, r: u32, rd: Renderer| open_cpal(d, r, rd)),
        "wasapi" => Box::new(|d: Option<&str>, r: u32, rd: Renderer| open_wasapi_shared(d, r, rd)),
        "wasapi-excl" => Box::new(|d: Option<&str>, r: u32, rd: Renderer| open_wasapi_exclusive(d, r, rd)),
        _ => bail!("backend must be cpal | wasapi | wasapi-excl"),
    })
}

fn files(dir: &Path, fmt: &str, rate: u32) -> Result<(Vec<PathBuf>, usize, usize)> {
    let ext = match fmt {
        f if f.starts_with("wav") => "wav",
        f if f.starts_with("flac") => "flac",
        f if f.starts_with("mp3") => "mp3",
        _ => "m4a",
    };
    let split = rate as usize * 5 + 777;
    let total = rate as usize * 10;
    Ok((
        vec![dir.join(format!("{fmt}_{rate}_a.{ext}")), dir.join(format!("{fmt}_{rate}_b.{ext}"))],
        split,
        total,
    ))
}

pub struct Tick {
    pub t: f64,
    pub track: usize,
    pub ms: f64,
}

/// 10 Hz position reporter. Runs until `stop` returns true or the engine ends.
fn report_10hz(eng: &Engine, max: Duration, mut events: impl FnMut(&Engine, f64)) -> Vec<Tick> {
    let t0 = Instant::now();
    let mut ticks = vec![];
    let mut next = t0;
    loop {
        next += Duration::from_millis(100);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        let t = t0.elapsed().as_secs_f64();
        let p = eng.position();
        ticks.push(Tick { t, track: p.track, ms: p.ms });
        events(eng, t);
        if p.ended || t0.elapsed() > max {
            break;
        }
    }
    ticks
}

fn tick_stats(ticks: &[Tick]) -> String {
    let mut max_dt: f64 = 0.0;
    let mut min_dt: f64 = 1e9;
    let mut backwards = 0;
    for w in ticks.windows(2) {
        let dt = w[1].t - w[0].t;
        max_dt = max_dt.max(dt);
        min_dt = min_dt.min(dt);
        if w[1].track == w[0].track && w[1].ms + 1.0 < w[0].ms {
            backwards += 1;
        }
    }
    // Progress linearity: position delta per tick vs wall-clock delta, while playing in one track.
    let mut max_dev: f64 = 0.0;
    for w in ticks.windows(2) {
        if w[1].track == w[0].track && w[0].ms > 0.0 {
            let d = (w[1].ms - w[0].ms) - (w[1].t - w[0].t) * 1000.0;
            max_dev = max_dev.max(d.abs());
        }
    }
    format!(
        "{} ticks, interval {:.1}..{:.1} ms, position went backwards {} times, max |pos_step - wall_step| = {:.1} ms (within one track)",
        ticks.len(),
        min_dt * 1000.0,
        max_dt * 1000.0,
        backwards,
        max_dev
    )
}

fn left_of(rec: &[f32]) -> Vec<f32> {
    rec.chunks(2).map(|c| c[0]).collect()
}

/// Analyse a callback recording of the A|B playlist against the reference sweep.
fn analyse_gapless(rec: &[f32], reference: &[f32], split: usize, total: usize, rate: u32) -> String {
    let left = left_of(rec);
    let first = left.iter().position(|v| v.abs() > 1e-5).unwrap_or(0).saturating_sub(1);
    let dec = &left[first..];
    if dec.len() < total {
        return format!("recording too short: {} < {}", dec.len(), total);
    }
    let (lag_a, ca) = best_lag(dec, reference, 0, 3000);
    // Boundary: where does B start relative to where A's end predicts?
    let b_slice = &dec[(split as i64 + lag_a).max(0) as usize..];
    let (gap, cb) = best_lag(b_slice, reference, split, 3000);
    let (maxe_all, snr_all) = compare(dec, reference, 0, lag_a, 4000, total - 8000);
    let (maxe_bnd, _) = compare(dec, reference, 0, lag_a, split - 64, 128);
    // longest run of exact zeros in the played region (silence gap)
    let (mut run, mut best_run) = (0usize, 0usize);
    for v in &dec[4000..total - 4000] {
        if *v == 0.0 {
            run += 1;
            best_run = best_run.max(run);
        } else {
            run = 0;
        }
    }
    format!(
        "first audio at rec frame {first} | lag(A)={lag_a:+} corr {ca:.4} | BOUNDARY GAP = {gap:+} frames = {:.3} ms (corr {cb:.4}) | max err overall {maxe_all:.2e}, within +-64 frames of boundary {maxe_bnd:.2e}, SNR {snr_all:.1} dB | longest exact-zero run {best_run} frames",
        gap as f64 * 1000.0 / rate as f64
    )
}

pub fn cmd_gapless(dir: &Path, backend: &str, fmt: &str, rate: u32, device: Option<String>, trim_ovr: bool) -> Result<()> {
    let (paths, split, total) = files(dir, fmt, rate)?;
    let reference = load_ref(&dir.join(format!("ref_{rate}_full.wav")))?;
    let trims = if trim_ovr {
        vec![Some((0, split as u64)), Some((0, (total - split) as u64))]
    } else {
        vec![None, None]
    };
    let mem0 = sys::mem();
    let cpu0 = sys::cpu_seconds();
    let w0 = Instant::now();
    let mut eng = Engine::start(paths, trims, rate, device, true, Some(14.0), opener(backend)?)?;
    let be_name = eng.backend.as_ref().unwrap().name();
    let be_mode = eng.backend.as_ref().unwrap().mode();
    let ticks = report_10hz(&eng, Duration::from_secs(14), |_, _| {});
    let wall = w0.elapsed().as_secs_f64();
    let cpu = sys::cpu_seconds() - cpu0;
    let mem = sys::mem();
    let cold_ms = (eng.sh.first_audio_ns.load(Relaxed) as f64 - eng.sh.cmd_ns.load(Relaxed) as f64) / 1e6;
    let lat_ms = eng.sh.latency_frames.load(Relaxed) as f64 * 1000.0 / rate as f64;
    let cbs = eng.sh.callbacks.load(Relaxed);
    let cb_avg = eng.sh.cb_total_ns.load(Relaxed) as f64 / cbs.max(1) as f64 / 1000.0;
    let cb_max = eng.sh.cb_max_ns.load(Relaxed) as f64 / 1000.0;
    let under = eng.sh.underruns.load(Relaxed);
    let consumed = eng.sh.consumed.load(Relaxed);
    let rec = eng.rec.clone().unwrap();
    let logs = eng.stop();
    let rec = rec.lock().unwrap().clone();
    println!("[{backend}/{fmt}/{rate}{}] device={be_name} ({be_mode})", if trim_ovr { "+ext-trim" } else { "" });
    println!("   {}", analyse_gapless(&rec, &reference, split, total, rate));
    println!(
        "   cold start (Engine::start -> first real audio callback): {cold_ms:.1} ms; device/buffer latency reported: {lat_ms:.1} ms; underruns {under}; frames consumed {consumed} (expected {total})"
    );
    println!(
        "   callbacks {cbs}, render time avg {cb_avg:.1} us max {cb_max:.1} us | process CPU {:.2}% over {wall:.1} s | working set {:.1} -> {:.1} MB, private {:.1} MB",
        cpu / wall * 100.0,
        mem0.working_set_mb,
        mem.working_set_mb,
        mem.private_mb
    );
    println!("   10 Hz reporter: {}", tick_stats(&ticks));
    for l in logs {
        println!("   log: {l}");
    }
    Ok(())
}

/// Seek + volume test on a single backend/format: verifies seek correctness against the reference.
pub fn cmd_seekvol(dir: &Path, backend: &str, fmt: &str, rate: u32) -> Result<()> {
    let (paths, split, total) = files(dir, fmt, rate)?;
    let reference = load_ref(&dir.join(format!("ref_{rate}_full.wav")))?;
    let eng = Engine::start(paths, vec![None, None], rate, None, true, Some(16.0), opener(backend)?)?;
    let mut seek_log: Vec<(f64, usize, u64, f64)> = vec![]; // (t, track, frame, issued_ns)
    let mut vol_log: Vec<(f64, f32)> = vec![];
    let mut seek_lat: Vec<f64> = vec![];
    let mut pending_seek: Option<f64> = None;
    let ticks = report_10hz(&eng, Duration::from_secs(10), |e, t| {
        // Script: seek to 3.0 s of A at t=1.0; vol 0.25 at 2.0; seek to B+1.0 s at 3.0; vol 1.0 at 4.0; seek A end-0.5 s at 5.0
        let act = |at: f64| t >= at && t < at + 0.1;
        if act(1.0) {
            e.seek(0, rate as u64 * 3);
            seek_log.push((t, 0, rate as u64 * 3, 0.0));
            pending_seek = Some(t);
        }
        if act(2.0) {
            e.set_volume(0.25);
            vol_log.push((t, 0.25));
        }
        if act(3.0) {
            e.seek(1, rate as u64);
            seek_log.push((t, 1, rate as u64, 0.0));
            pending_seek = Some(t);
        }
        if act(4.0) {
            e.set_volume(1.0);
            vol_log.push((t, 1.0));
        }
        if act(5.0) {
            e.seek(0, split as u64 - rate as u64 / 2);
            seek_log.push((t, 0, split as u64 - rate as u64 / 2, 0.0));
            pending_seek = Some(t);
        }
        if pending_seek.is_some() && e.sh.first_audio_ns.load(Relaxed) != 0 {
            let l = (e.sh.first_audio_ns.load(Relaxed) as f64 - e.sh.cmd_ns.load(Relaxed) as f64) / 1e6;
            seek_lat.push(l);
            pending_seek = None;
        }
    });
    let lat_ms = eng.sh.latency_frames.load(Relaxed) as f64 * 1000.0 / rate as f64;
    let cb_max = eng.sh.cb_max_ns.load(Relaxed) as f64 / 1000.0;
    let rec = eng.rec.clone().unwrap();
    let logs = eng.stop();
    let rec = rec.lock().unwrap().clone();
    println!("[{backend}/{fmt}/{rate}] seek+volume test (device latency {lat_ms:.1} ms, max render {cb_max:.1} us)");
    println!("   seek-to-first-audio latencies (ms, command -> first callback with data): {:?}", seek_lat.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>());
    println!("   reporter: {}", tick_stats(&ticks));
    // Verify seeks: the recording must contain the reference audio from the seek target on.
    let left = left_of(&rec);
    let rms = |s: &[f32]| (s.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / s.len().max(1) as f64).sqrt();
    let _ = (total, &vol_log);
    for (i, (t_seek, track, frame, _)) in seek_log.iter().enumerate() {
        let ref_off = if *track == 0 { *frame as usize } else { split + *frame as usize };
        let needle = &reference[ref_off + 3000..ref_off + 9000];
        let nr = rms(needle);
        let corr_at = |idx: usize| -> f64 {
            let (mut c, mut ed) = (0.0f64, 0.0f64);
            for k in 0..6000 {
                let d = left[idx + k] as f64;
                c += d * needle[k] as f64;
                ed += d * d;
            }
            c / (ed.sqrt() * nr * (6000f64).sqrt() + 1e-12)
        };
        // coarse search (stride 16, subsampled) then exact refinement
        let mut best = (0usize, -2.0f64);
        let lo = ((*t_seek * rate as f64) as usize).saturating_sub(4000);
        let hi = (lo + 40000).min(left.len() - 6000);
        let mut idx = lo;
        while idx < hi {
            let (mut c, mut ed) = (0.0f64, 0.0f64);
            for k in 0..2000 {
                let d = left[idx + k] as f64;
                c += d * needle[k] as f64;
                ed += d * d;
            }
            let n = c / (ed.sqrt() * nr * 1.0 + 1e-12);
            if n > best.1 {
                best = (idx, n);
            }
            idx += 1;
        }
        let _ = &corr_at;
        // refine by least squares with a free gain (exact-copy detection)
        let mut refined = (best.0, f64::MAX);
        for idx in best.0.saturating_sub(12)..(best.0 + 12).min(hi) {
            let (mut c, mut nn) = (0.0f64, 0.0f64);
            for k in 0..6000 {
                c += left[idx + k] as f64 * needle[k] as f64;
                nn += (needle[k] as f64).powi(2);
            }
            let g = c / nn;
            let mut e = 0.0f64;
            for k in 0..6000 {
                e += (left[idx + k] as f64 - g * needle[k] as f64).powi(2);
            }
            if e < refined.1 {
                refined = (idx, e);
            }
        }
        let g = rms(&left[refined.0..refined.0 + 6000]) / nr;
        let maxd = (0..6000).map(|k| (left[refined.0 + k] - g as f32 * needle[k]).abs()).fold(0.0f32, f32::max);

        println!(
            "   seek#{i} -> track {track} frame {frame}: audio found at recording frame {} = {:.0} ms after the command ; max |recorded - gain*reference| over 6000 frames = {maxd:.1e}; gain {g:.3}",
            refined.0 as i64 - 3000,
            (refined.0 as f64 - 3000.0) * 1000.0 / rate as f64 - t_seek * 1000.0,
        );
    }
    // Volume ramp smoothness: largest sample-to-sample step of the recording around the 0.25 and 1.0 changes
    // relative to the signal's own largest step (a click would exceed it).
    let steps = |s: &[f32]| s.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    println!("   largest sample step in whole recording (clicks would show as outliers vs 2*pi*f/fs*amp): {:.4}", steps(&left));
    for l in logs {
        println!("   log: {l}");
    }
    Ok(())
}

/// Device switch while playing: enumerate endpoints, move to every other endpoint and back.
pub fn cmd_switch(dir: &Path, backend: &str, rate: u32) -> Result<()> {
    let devs = list_wasapi_devices()?;
    println!("render endpoints:");
    for d in &devs {
        println!("  {} {} [{}] id={}", if d.is_default { "*" } else { " " }, d.name, d.mix, d.id);
    }
    let reference = load_ref(&dir.join(format!("ref_{rate}_full.wav")))?;
    let _ = reference;
    let (paths, _split, _total) = files(dir, "wav16", rate)?;
    let mut eng = Engine::start(paths, vec![None, None], rate, None, true, Some(16.0), opener(backend)?)?;
    std::thread::sleep(Duration::from_millis(1500));
    let p0 = eng.position();
    println!("   playing on default at {:.0} ms", p0.ms);
    let others: Vec<&DevInfo> = devs.iter().filter(|d| !d.is_default).collect();
    if others.is_empty() {
        println!("   only one render endpoint present: cannot switch to a different endpoint on this machine.");
    }
    // Switch default -> each other -> default; measure time and position continuity.
    let mut targets: Vec<(String, Option<String>)> = others.iter().map(|d| (d.name.clone(), Some(d.id.clone()))).collect();
    targets.push(("(system default)".into(), None));
    for (name, id) in targets {
        let before = eng.position();
        match eng.switch_device(id.clone()) {
            Ok(d) => {
                std::thread::sleep(Duration::from_millis(600));
                let after = eng.position();
                println!(
                    "   -> {name}: switch took {:.0} ms (stop old + open new + first audio); position before {:.0} ms, 600 ms after switch {:.0} ms (expected ~ +{:.0} + switch)",
                    d.as_secs_f64() * 1000.0,
                    before.ms,
                    after.ms,
                    600.0
                );
            }
            Err(e) => println!("   -> {name}: FAILED: {e:#}"),
        }
    }
    let logs = eng.stop();
    for l in logs {
        println!("   log: {l}");
    }
    Ok(())
}

/// Wait for default-device change / loss notifications on the given backend (for the PO runbook).
pub fn cmd_follow(dir: &Path, backend: &str, rate: u32, secs: u64) -> Result<()> {
    let watcher = DefaultWatcher::start(true)?;
    let (paths, _s, _t) = files(dir, "wav16", rate)?;
    // loop the pair many times so there is something to hear
    let mut all = vec![];
    for _ in 0..40 {
        all.extend(paths.clone());
    }
    let n = all.len();
    let mut eng = Engine::start(all, vec![None; n], rate, None, true, None, opener(backend)?)?;
    println!("playing on [{}] ({}), waiting {secs} s for device events; unplug/switch the default device now...", eng.backend.as_ref().unwrap().name(), eng.backend.as_ref().unwrap().mode());
    let t0 = Instant::now();
    while t0.elapsed().as_secs() < secs {
        std::thread::sleep(Duration::from_millis(100));
        let lost = eng.backend.as_ref().map(|b| b.lost()).unwrap_or(false);
        if watcher.changed.swap(false, SeqCst) || lost {
            let t = Instant::now();
            let p = eng.position();
            println!("  event at {:.1}s (backend lost={lost}); position {:.0} ms; rebuilding on default...", t0.elapsed().as_secs_f64(), p.ms);
            match eng.switch_device(None) {
                Ok(d) => println!("  recovered in {:.0} ms on [{}]; resumed at track {} {:.0} ms", d.as_secs_f64() * 1000.0, eng.backend.as_ref().unwrap().name(), p.track, p.ms),
                Err(e) => println!("  recovery FAILED after {:.0} ms: {e:#}", t.elapsed().as_secs_f64() * 1000.0),
            }
        }
    }
    eng.stop();
    Ok(())
}

pub fn cmd_cpu(dir: &Path, backend: &str, fmt: &str, rate: u32, secs: f64) -> Result<()> {
    let ext = match fmt {
        f if f.starts_with("wav") => "wav",
        f if f.starts_with("flac") => "flac",
        f if f.starts_with("mp3") => "mp3",
        _ => "m4a",
    };
    let p = dir.join(format!("long_{fmt}_{rate}.{ext}"));
    if !p.exists() {
        bail!("missing {}; run genlong", p.display());
    }
    let mem0 = sys::mem();
    let eng = Engine::start(vec![p], vec![None], rate, None, true, None, opener(backend)?)?;
    std::thread::sleep(Duration::from_millis(1500)); // settle: startup bursts out of the way
    let c0 = sys::cpu_seconds();
    let w0 = Instant::now();
    std::thread::sleep(Duration::from_secs_f64(secs));
    let wall = w0.elapsed().as_secs_f64();
    let cpu = sys::cpu_seconds() - c0;
    let mem = sys::mem();
    let under = eng.sh.underruns.load(Relaxed);
    let cbs = eng.sh.callbacks.load(Relaxed);
    let cb_avg = eng.sh.cb_total_ns.load(Relaxed) as f64 / cbs.max(1) as f64 / 1000.0;
    let cb_max = eng.sh.cb_max_ns.load(Relaxed) as f64 / 1000.0;
    eng.stop();
    println!(
        "[{backend}/{fmt}/{rate}] steady-state CPU {:.2}% of one core over {wall:.0} s | working set {:.1} MB (peak {:.1}), private {:.1} MB (baseline before engine {:.1} MB) | render avg {cb_avg:.1} us max {cb_max:.1} us | underruns {under}",
        cpu / wall * 100.0,
        mem.working_set_mb,
        mem.peak_working_set_mb,
        mem.private_mb,
        mem0.working_set_mb
    );
    Ok(())
}

pub fn cmd_excl(rate: u32) -> Result<()> {
    use wasapi::*;
    let _ = initialize_mta();
    let en = DeviceEnumerator::new().map_err(|e| anyhow!("{e}"))?;
    let dev = en.get_default_device(&Direction::Render).map_err(|e| anyhow!("{e}"))?;
    println!("default endpoint: {}", dev.get_friendlyname().unwrap_or_default());
    let client = dev.get_iaudioclient().map_err(|e| anyhow!("{e}"))?;
    let mix = client.get_mixformat().map_err(|e| anyhow!("{e}"))?;
    println!("shared-mode mix format: {mix:?}");
    let (def, min) = client.get_device_period().map_err(|e| anyhow!("{e}"))?;
    println!("device period default {} us, minimum {} us", def / 10, min / 10);
    println!("exclusive-mode IsFormatSupported matrix (rate x format):");
    for r in [44100usize, 48000, 88200, 96000, 176400, 192000] {
        let mut ok = vec![];
        for (bits, vbits, ty, label) in [
            (16, 16, SampleType::Int, "i16"),
            (24, 24, SampleType::Int, "i24 packed"),
            (32, 24, SampleType::Int, "i24 in 32"),
            (32, 32, SampleType::Int, "i32"),
            (32, 32, SampleType::Float, "f32"),
        ] {
            let f = WaveFormat::new(bits, vbits, &ty, r, 2, None);
            if client.is_supported(&f, &ShareMode::Exclusive).is_ok() {
                ok.push(label);
            }
        }
        println!("   {r:>6} Hz: {}", if ok.is_empty() { "none".into() } else { ok.join(", ") });
    }
    // Play 1.5 s of -20 dBFS tone exclusively through the engine path (opens, starts, stops).
    println!("opening exclusive stream at {rate} Hz (briefly takes the device away from other apps)...");
    drop(client);
    let dir = Path::new("out");
    let (paths, _s, _t) = files(dir, "wav16", rate)?;
    let t0 = Instant::now();
    match Engine::start(paths, vec![None, None], rate, None, true, None, opener("wasapi-excl")?) {
        Ok(eng) => {
            std::thread::sleep(Duration::from_millis(1500));
            let p = eng.position();
            let b = eng.backend.as_ref().unwrap();
            println!(
                "   exclusive OK on [{}], mode {}; position after 1.5 s = {:.0} ms; underruns {}; open+first audio {:.0} ms",
                b.name(),
                b.mode(),
                p.ms,
                p.underruns,
                (eng.sh.first_audio_ns.load(Relaxed) as f64 - eng.sh.cmd_ns.load(Relaxed) as f64) / 1e6
            );
            let _ = t0;
            eng.stop();
        }
        Err(e) => println!("   exclusive stream FAILED: {e:#}"),
    }
    Ok(())
}
