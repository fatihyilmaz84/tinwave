//! Runs the audio providers for a while so CPU and memory can be measured from outside.
//!
//! ```text
//! cargo run --release -p tinwave-context --example audio_probe -- 30
//! ```
//!
//! Phase 1 (3 s): the signal bus exists, the provider is not started: the baseline. Phase 2 (the argument, in seconds, default 30):
//! the provider thread is running and idle. Sample the printed PID in between, e.g. with
//! `Get-Process -Id <pid>` (`TotalProcessorTime`, `WorkingSet64`).
//!
//! It only listens: it never changes the system volume or mute.

use std::sync::Arc;
use std::time::Duration;

use tinwave_context::audio::{
    AudioMonitor, AudioMonitorConfig, DeviceSecret, OutputTarget, PlaybackContextSource,
    PlaybackSnapshot,
};
use tinwave_context::{ContextFields, ContextProvider};
use tinwave_core::{DeviceId, SystemClock, SystemRng};
use tinwave_signals::{BusConfig, SignalBus, SignalSettings};

/// Nothing plays, so nothing is emitted; the provider still tracks everything.
struct Silent;

impl PlaybackContextSource for Silent {
    fn current(&self) -> Option<PlaybackSnapshot> {
        None
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seconds: u64 = std::env::args().nth(1).map_or(Ok(30), |a| a.parse())?;
    // The bus exists in the app with or without the provider, so it is part of the baseline.
    let device = DeviceId::generate(&mut tinwave_core::IdGenerator::system())?;
    let (bus, _drain) = SignalBus::channel(
        BusConfig::new(device),
        SystemClock,
        Arc::new(SignalSettings::new()),
    )?;
    let secret = DeviceSecret::generate(&mut SystemRng)?;
    println!("pid {}", std::process::id());
    println!("phase 1: baseline, bus created, provider not started (3 s)");
    std::thread::sleep(Duration::from_secs(3));

    let monitor = AudioMonitor::start(AudioMonitorConfig {
        bus,
        secret,
        playback: Arc::new(Silent),
        target: OutputTarget::SystemDefault,
    })?;
    println!("phase 2: provider running ({seconds} s)");
    std::thread::sleep(Duration::from_secs(1));
    let mut fields = ContextFields::default();
    monitor.output_device().snapshot(&mut fields);
    monitor.system_volume().snapshot(&mut fields);
    println!("fields: {fields:?}");
    std::thread::sleep(Duration::from_secs(seconds));
    println!("stats: {:?}", monitor.stats());
    Ok(())
}
