//! Dev-only teardown stress target for the artwork image provider (ADR 0008, image provider lifetime, rule 6).
//!
//! It runs the app's own startup and teardown (`Shell`: engine, provider, `UiOwner`, explicit shutdown after `exec()`)
//! around a stress scene instead of `Main.qml`, with a synthetic artwork source that takes a few milliseconds per
//! thumbnail so requests really are in flight when the scene quits. Not packaged with the app.
//!
//! ```text
//! image-stress [--qml=<path>] [--pool=<threads>] [--delay-ms=<ms>] [--exit-ms=<ms>] [--mode=<name>]
//! ```
//!
//! `--qml` defaults to `Storm.qml` next to this file; `--pool` (default 1, the setting that exposed the T-0041 crash
//! most) and `--delay-ms` (default 3) configure the provider and the source; the remaining arguments are read by the
//! scene itself (`Qt.application.arguments`). On success it prints, in this order:
//!
//! 1. `STRESS_DONE exit=<code> responses=<n> finished=<n> ...` after the event loop ended and the providers were
//!    quiesced, with the provider's counters;
//! 2. `STRESS_TEARDOWN_COMPLETE` after the engine was destroyed.
//!
//! `tools/stress/run-stress.ps1` runs it many times with random quit times and counts crashes and hangs.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tinwave_desktop::app::wiring::{Shell, ShellConfig};
use tinwave_desktop::image::ArtworkSource;

/// Gradient thumbnails after a fixed delay (the stand-in for reading and decoding a cover). Keys starting with
/// `missing` have no artwork, to exercise the error path.
#[derive(Debug)]
struct SyntheticArtwork {
    delay: Duration,
}

impl ArtworkSource for SyntheticArtwork {
    fn thumbnail(&self, key: &str, width: u32, height: u32) -> Option<Vec<u8>> {
        std::thread::sleep(self.delay);
        if key.starts_with("missing") {
            return None;
        }
        let seed = key
            .bytes()
            .fold(17u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
        let (w, h) = (width as usize, height as usize);
        let mut pixels = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                // Bytes are B, G, R, opaque (QImage::Format_RGB32 on little-endian).
                let shade = ((x + y) * 255 / (w + h)) as u32;
                pixels.extend_from_slice(&[
                    (seed >> 16) as u8,
                    (seed >> 8) as u8 ^ shade as u8,
                    seed as u8,
                    0xFF,
                ]);
            }
        }
        Some(pixels)
    }
}

fn arg_value(name: &str) -> Option<String> {
    let prefix = format!("--{name}=");
    std::env::args().find_map(|a| a.strip_prefix(&prefix).map(str::to_owned))
}

fn main() -> ExitCode {
    let qml_path = arg_value("qml").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/stress/Storm.qml"),
        PathBuf::from,
    );
    let pool = arg_value("pool")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_i32);
    let delay = Duration::from_millis(
        arg_value("delay-ms")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3),
    );

    let config = ShellConfig {
        qml_url: format!(
            "file:///{}",
            qml_path.display().to_string().replace('\\', "/")
        ),
        artwork: Arc::new(SyntheticArtwork { delay }),
        artwork_pool_threads: pool,
        select_style: false,
    };
    let mut shell = match Shell::start(config) {
        Ok(shell) => shell,
        Err(code) => return code,
    };
    let code = shell.exec();
    shell.shutdown();

    let s = shell.artwork().stats();
    println!(
        "STRESS_DONE exit={code} responses={} finished={} duplicate_finishes={} cancels={} tasks_run={} \
         tasks_skipped={} requests={} cache_hits={} source_calls={} misses={}",
        s.responses,
        s.finished,
        s.duplicate_finishes,
        s.cancels,
        s.tasks_run,
        s.tasks_skipped,
        s.core.requests,
        s.core.cache_hits,
        s.core.source_calls,
        s.core.misses,
    );
    drop(shell);
    println!("STRESS_TEARDOWN_COMPLETE");
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
