//! rubato (MIT/Apache-2.0) feasibility: 44.1 -> 48 kHz across a track boundary.
use crate::gen::sweep;
use anyhow::{anyhow, Result};
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::time::Instant;

fn run_one(input: &[f32], rin: usize, rout: usize) -> Result<Vec<f32>> {
    let mut rs = Fft::<f32>::new(rin, rout, 1024, 2, FixedSync::Input).map_err(|e| anyhow!("{e}"))?;
    let frames = input.len() / 2;
    let ad = InterleavedSlice::new(input, 2, frames).map_err(|e| anyhow!("{e}"))?;
    let out = rs.process_all(&ad, frames, None).map_err(|e| anyhow!("{e}"))?;
    Ok(out.take_data())
}

pub fn run() -> Result<()> {
    let (rin, rout) = (44100usize, 48000usize);
    let total = rin * 10;
    let split = rin * 5 + 777;
    let (l, r) = sweep(rin as u32, total, 200.0, 12000.0);
    let mut whole = Vec::with_capacity(total * 2);
    for i in 0..total {
        whole.push(l[i] as f32);
        whole.push(r[i] as f32);
    }
    let t0 = Instant::now();
    let w = run_one(&whole, rin, rout)?;
    let dt = t0.elapsed().as_secs_f64();
    println!("rubato Fft 44100->48000: {} s of stereo audio resampled in {:.1} ms ({:.0}x realtime, {:.2}% of one core when streaming)", total / rin, dt * 1000.0, (total / rin) as f64 / dt, dt / (total / rin) as f64 * 100.0);
    let a = run_one(&whole[..split * 2], rin, rout)?;
    let b = run_one(&whole[split * 2..], rin, rout)?;
    println!("output frames: whole {} | A {} + B {} = {} (expected ceil ratio: {} + {})", w.len() / 2, a.len() / 2, b.len() / 2, (a.len() + b.len()) / 2,
        (split as f64 * rout as f64 / rin as f64).ceil(), ((total - split) as f64 * rout as f64 / rin as f64).ceil());
    let mut cat = a.clone();
    cat.extend(&b);
    let n = cat.len().min(w.len());
    let mut worst_bnd = 0.0f32;
    let mut worst_other = 0.0f32;
    let bnd = a.len() / 2;
    for i in 0..n / 2 {
        let e = (cat[2 * i] - w[2 * i]).abs();
        if (i as i64 - bnd as i64).abs() < 400 {
            worst_bnd = worst_bnd.max(e);
        } else {
            worst_other = worst_other.max(e);
        }
    }
    println!("independent per-track resampling vs one continuous resampler: max error within +-400 frames of boundary {worst_bnd:.2e}, elsewhere {worst_other:.2e}");
    println!("  (a continuous resampler across same-rate tracks has no such error; independent ones only matter at rate changes)");
    Ok(())
}
