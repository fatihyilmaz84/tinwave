mod backends;
mod bench;
mod engine;
mod gen;
mod mf_aac;
mod probe;
mod reader;
mod resamp;
mod sys;
mod sysvol;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

thread_local! {
    /// Set while a thread is inside the audio callback (`Renderer::render`).
    pub static IN_RT: Cell<bool> = const { Cell::new(false) };
}
pub static RT_ALLOCS: AtomicU64 = AtomicU64::new(0);

struct CountingAlloc;
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if IN_RT.try_with(|c| c.get()).unwrap_or(false) {
            RT_ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if IN_RT.try_with(|c| c.get()).unwrap_or(false) {
            RT_ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if IN_RT.try_with(|c| c.get()).unwrap_or(false) {
            RT_ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let a = |i: usize, d: &str| args.get(i).cloned().unwrap_or_else(|| d.to_string());
    let dir = PathBuf::from("out");
    match args.get(1).map(String::as_str) {
        Some("gen") => {
            for rate in [48000u32, 44100] {
                let split = (rate as usize) * 5 + 777;
                gen::generate(&dir, rate, 10.0, split)?;
                println!("generated rate {rate} split {split}");
            }
        }
        Some("genlong") => {
            for rate in [48000u32, 44100] {
                let (l, r) = gen::sweep(rate, rate as usize * 40, 200.0, 12000.0);
                gen::write_wav16(&dir.join(format!("long_wav16_{rate}.wav")), rate, &l, &r)?;
                gen::write_flac(&dir.join(format!("long_flac16_{rate}.flac")), rate, 16, &l, &r)?;
                gen::write_flac(&dir.join(format!("long_flac24_{rate}.flac")), rate, 24, &l, &r)?;
                gen::write_mp3(&dir.join(format!("long_mp3lame_{rate}.mp3")), rate, &l, &r, true)?;
                gen::write_alac_m4a(&dir.join(format!("long_alac_{rate}.m4a")), rate, &l, &r)?;
                mf_aac::write_aac_m4a(&dir.join(format!("long_aac_{rate}.m4a")), rate, &l, &r)?;
            }
        }
        Some("probe") => {
            for rate in [48000u32, 44100] {
                probe::run(&dir, rate)?;
            }
        }
        Some("devices") => {
            println!("WASAPI render endpoints (wasapi crate):");
            for d in backends::list_wasapi_devices()? {
                println!("  {} {}  [{}]\n      id={}", if d.is_default { "*" } else { " " }, d.name, d.mix, d.id);
            }
            println!("cpal devices:");
            backends::list_cpal_devices()?;
        }
        // gapless <backend> <fmt> <rate> [ext-trim]
        Some("gapless") => bench::cmd_gapless(&dir, &a(2, "cpal"), &a(3, "wav16"), a(4, "48000").parse()?, None, a(5, "") == "ext-trim")?,
        // seekvol <backend> <fmt> <rate>
        Some("seekvol") => bench::cmd_seekvol(&dir, &a(2, "cpal"), &a(3, "wav16"), a(4, "48000").parse()?)?,
        Some("switch") => bench::cmd_switch(&dir, &a(2, "cpal"), a(3, "48000").parse()?)?,
        Some("follow") => bench::cmd_follow(&dir, &a(2, "cpal"), a(3, "48000").parse()?, a(4, "60").parse()?)?,
        // cpu <backend> <fmt> <rate> [secs]
        Some("cpu") => bench::cmd_cpu(&dir, &a(2, "cpal"), &a(3, "flac16"), a(4, "48000").parse()?, a(5, "10").parse()?)?,
        Some("loopback") => bench::cmd_loopback(&dir, &a(2, "cpal"), &a(3, "flac16"), a(4, "48000").parse()?)?,
        Some("decspeed") => probe::decode_speed(&dir, a(2, "48000").parse()?)?,
        Some("sysvol") => sysvol::run(a(2, "0").parse()?)?,
        Some("play") => bench::cmd_play(&a(2, "wasapi"), a(3, "0.1").parse()?, args.iter().skip(4).map(PathBuf::from).collect())?,
        Some("resamp") => resamp::run()?,
        Some("excl") => bench::cmd_excl(a(2, "48000").parse()?)?,
        _ => eprintln!("usage: gen | genlong | probe | devices | gapless | seekvol | switch | follow | cpu | excl"),
    }
    Ok(())
}
