//! Offline format matrix: decode / length / alignment / seek accuracy per format.
use crate::reader::TrackReader;
use anyhow::Result;
use std::path::Path;
use std::time::Instant;

pub fn load_ref(path: &Path) -> Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let v: Vec<f32> = r.samples::<f32>().collect::<std::result::Result<_, _>>()?;
    Ok(v.chunks(2).map(|c| c[0]).collect()) // left channel
}

pub struct Decoded {
    pub left: Vec<f32>,
    pub info: String,
}

pub fn decode_all(path: &Path, gapless: bool) -> Result<Decoded> {
    decode_all_ovr(path, gapless, None)
}

pub fn decode_all_ovr(path: &Path, gapless: bool, ovr: Option<(u64, u64)>) -> Result<Decoded> {
    let mut rd = TrackReader::open(path, gapless)?;
    if let Some((d, v)) = ovr {
        rd.set_gapless_override(d, v);
    }
    let mut buf = Vec::new();
    while rd.next_chunk(&mut buf)? {}
    let left = buf.chunks(2).map(|c| c[0]).collect();
    let info = format!(
        "{} {}Hz {}ch num_frames={:?} delay={:?} padding={:?}",
        rd.codec, rd.rate, rd.src_channels, rd.total_frames, rd.delay, rd.padding
    );
    Ok(Decoded { left, info })
}

/// Find lag such that dec[i + lag] ~ reference[ref_off + i]. Returns (lag, normalised correlation).
pub fn best_lag(dec: &[f32], reference: &[f32], ref_off: usize, search: i64) -> (i64, f64) {
    let start = 3000usize; // skip the very quiet start of the sweep
    let n = 8000usize;
    let mut best = (0i64, f64::MIN);
    let er: f64 = (0..n).map(|i| (reference[ref_off + start + i] as f64).powi(2)).sum();
    for lag in -search..=search {
        let mut c = 0.0f64;
        let mut ed = 0.0f64;
        let mut ok = true;
        for i in 0..n {
            let di = (start + i) as i64 + lag;
            if di < 0 || di as usize >= dec.len() {
                ok = false;
                break;
            }
            let d = dec[di as usize] as f64;
            c += d * reference[ref_off + start + i] as f64;
            ed += d * d;
        }
        if !ok {
            continue;
        }
        let norm = c / (er.sqrt() * ed.sqrt() + 1e-12);
        if norm > best.1 {
            best = (lag, norm);
        }
    }
    best
}

/// Max abs error and SNR (dB) of dec vs reference at a given lag over `len` samples starting at ref_off+from.
pub fn compare(dec: &[f32], reference: &[f32], ref_off: usize, lag: i64, from: usize, len: usize) -> (f64, f64) {
    let (mut maxe, mut se, mut sr) = (0.0f64, 0.0f64, 0.0f64);
    for i in from..from + len {
        let di = i as i64 + lag;
        if di < 0 || di as usize >= dec.len() || ref_off + i >= reference.len() {
            break;
        }
        let r = reference[ref_off + i] as f64;
        let e = dec[di as usize] as f64 - r;
        maxe = maxe.max(e.abs());
        se += e * e;
        sr += r * r;
    }
    (maxe, 10.0 * (sr / se.max(1e-30)).log10())
}

pub struct FormatRow {
    pub name: String,
    pub text: String,
}

fn probe_format(dir: &Path, name: &str, ext: &str, rate: u32, split: usize, total: usize, gapless: bool, reference: &[f32], trim_to_valid: bool) -> String {
    let pa = dir.join(format!("{name}_{rate}_a.{ext}"));
    let pb = dir.join(format!("{name}_{rate}_b.{ext}"));
    let (exp_a, exp_b) = (split as i64, (total - split) as i64);
    let da = match decode_all_ovr(&pa, gapless, trim_to_valid.then_some((0, exp_a as u64))) {
        Ok(d) => d,
        Err(e) => return format!("DECODE FAIL: {e:#}"),
    };
    let db = match decode_all_ovr(&pb, gapless, trim_to_valid.then_some((0, exp_b as u64))) {
        Ok(d) => d,
        Err(e) => return format!("DECODE FAIL (B): {e:#}"),
    };
    let (lag_a, ca) = best_lag(&da.left, reference, 0, 3000);
    let (lag_b, cb) = best_lag(&db.left, reference, split, 3000);
    let len_err_a = da.left.len() as i64 - exp_a;
    let len_err_b = db.left.len() as i64 - exp_b;
    // Audible gap at the boundary: silence/offset carried at A's tail + B's head.
    let tail_excess_a = len_err_a - lag_a; // frames after the last real sample of A
    let gap = tail_excess_a + lag_b;
    let (maxe_a, snr_a) = compare(&da.left, reference, 0, lag_a, 5000, split.saturating_sub(10000));
    let (maxe_b, snr_b) = compare(&db.left, reference, split, lag_b, 5000, (total - split).saturating_sub(10000));
    format!(
        "{}\n      lenErr A={len_err_a:+} B={len_err_b:+} | lag A={lag_a:+} B={lag_b:+} (corr {ca:.4}/{cb:.4}) | boundary gap = {gap:+} frames ({:.3} ms) | maxErr A={maxe_a:.2e} B={maxe_b:.2e} | SNR A={snr_a:.1} dB B={snr_b:.1} dB",
        da.info,
        gap as f64 * 1000.0 / rate as f64
    )
}

fn probe_seek(path: &Path, rate: u32, reference: &[f32], gapless: bool) -> String {
    let mut out = String::new();
    let mut rd = match TrackReader::open(path, gapless) {
        Ok(r) => r,
        Err(e) => return format!("open fail: {e:#}"),
    };
    for target_frames in [0u64, 12_345, rate as u64 * 2 + 7, rate as u64 * 4, (5 * rate as u64) - 25000] {
        let t0 = Instant::now();
        if let Err(e) = rd.seek(target_frames) {
            out += &format!(" seek{target_frames}:FAIL({e:#})");
            continue;
        }
        let mut buf = Vec::new();
        while buf.len() < 2 * 12000 {
            match rd.next_chunk(&mut buf) {
                Ok(true) => {}
                _ => break,
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let left: Vec<f32> = buf.chunks(2).map(|c| c[0]).collect();
        // expected: left[k] == reference[target + k]. Need ref_off + 3000 + 8000 inside the buffer.
        if left.len() < 12000 || target_frames as usize + 11000 > reference.len() {
            out += &format!(" seek{target_frames}:short({})", left.len());
            continue;
        }
        let (lag, c) = best_lag(&left, reference, target_frames as usize, 3000);
        out += &format!(" [{target_frames}: off={lag:+} c={c:.3} {ms:.1}ms]");
    }
    out
}

pub fn run(dir: &Path, rate: u32) -> Result<()> {
    let reference = load_ref(&dir.join(format!("ref_{rate}_full.wav")))?;
    let split = rate as usize * 5 + 777;
    let total = rate as usize * 10;
    println!("=== format matrix @ {rate} Hz (A = {split} frames, B = {} frames) ===", total - split);
    let formats: &[(&str, &str, &str)] = &[
        ("wav16", "wav", "WAV PCM16"),
        ("wav24", "wav", "WAV PCM24"),
        ("wavf32", "wav", "WAV float32"),
        ("flac16", "flac", "FLAC 16"),
        ("flac24", "flac", "FLAC 24"),
        ("mp3lame", "mp3", "MP3 192k (LAME/Xing tag)"),
        ("mp3notag", "mp3", "MP3 192k (no gapless tag)"),
        ("alac", "m4a", "ALAC 16 (own muxer)"),
        ("aac", "m4a", "AAC-LC 192k (MF encoder)"),
    ];
    for (name, ext, label) in formats {
        println!("-- {label}  (gapless=true)");
        println!("   decode: {}", probe_format(dir, name, ext, rate, split, total, true, &reference, false));
        if matches!(*name, "mp3lame" | "aac") {
            println!("   decode gapless=false: {}", probe_format(dir, name, ext, rate, split, total, false, &reference, false));
        }
        if *name == "aac" {
            println!("   decode + external trim (delay=0, valid=exact; what an iTunSMPB/elst-aware library layer would pass): {}", probe_format(dir, name, ext, rate, split, total, true, &reference, true));
        }
        let pa = dir.join(format!("{name}_{rate}_a.{ext}"));
        println!("   seek:{}", probe_seek(&pa, rate, &reference, true));
    }
    Ok(())
}

/// Pure decode speed (no output): x realtime on one core, per codec.
pub fn decode_speed(dir: &Path, rate: u32) -> Result<()> {
    println!("decode throughput, 40 s stereo files @ {rate} Hz, single thread, release build:");
    for (name, ext) in [("wav16", "wav"), ("flac16", "flac"), ("flac24", "flac"), ("mp3lame", "mp3"), ("alac", "m4a"), ("aac", "m4a")] {
        let p = dir.join(format!("long_{name}_{rate}.{ext}"));
        if !p.exists() {
            continue;
        }
        let t0 = Instant::now();
        let mut rd = TrackReader::open(&p, true)?;
        let open_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut buf = Vec::new();
        let mut frames = 0usize;
        while rd.next_chunk(&mut buf)? {
            frames += buf.len() / 2;
            buf.clear();
        }
        let dt = t0.elapsed().as_secs_f64();
        println!("   {name:8} open {open_ms:5.1} ms; {frames} frames in {:.0} ms = {:.0}x realtime ({:.2}% of one core for playback)", dt * 1000.0, frames as f64 / rate as f64 / dt, dt / (frames as f64 / rate as f64) * 100.0);
    }
    Ok(())
}
