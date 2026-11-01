//! T-0008 spike: Windows system-volume and output-device change notifications.
//!
//! Commands:
//!   enumerate                 list all render+capture endpoints with form factor / transport / class
//!   watch [seconds]           log volume/mute/device events until Ctrl+C (or N seconds); for the PO runbook
//!   bench [seconds]           idle overhead: register everything, do nothing, report CPU + RSS
//!   selftest                  programmatic verification (self vs external classification, latency, burst)
//!   poke <ctx> <scalar|mute0|mute1>   helper child process used by selftest (ctx = null|new)

mod classify;
mod device;
mod monitor;
mod selftest;
mod sys;

use monitor::{Event, Monitor, TINWAVE_CTX};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::core::GUID;
use windows::Win32::Media::Audio::{eCapture, eRender};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

type R<T> = Result<T, Box<dyn std::error::Error>>;

pub fn origin(ctx: &GUID) -> String {
    if *ctx == TINWAVE_CTX {
        "self".into()
    } else if *ctx == GUID::zeroed() {
        "external(null-ctx)".into()
    } else {
        format!("external({ctx:?})")
    }
}

pub fn wall_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn print_event(e: &Event, t0_qpc: i64) {
    let rel = |q: i64| sys::qpc_to_ms(q - t0_qpc);
    match e {
        Event::Volume { qpc, ctx, volume, muted, channels, endpoint } => println!(
            "[{wall} +{rel:9.3}ms] VOLUME vol={volume:.3} muted={muted} ch={channels} origin={o} endpoint=..{ep}",
            wall = wall_ms(),
            rel = rel(*qpc),
            o = origin(ctx),
            ep = &endpoint[endpoint.len().saturating_sub(12)..],
        ),
        Event::DefaultChanged { qpc, flow, role, id } => println!(
            "[{} +{:9.3}ms] DEFAULT_CHANGED flow={} role={} id=..{}",
            wall_ms(),
            rel(*qpc),
            ["render", "capture"].get(*flow as usize).unwrap_or(&"?"),
            ["console", "multimedia", "communications"].get(*role as usize).unwrap_or(&"?"),
            &id[id.len().saturating_sub(12)..]
        ),
        Event::State { qpc, id, state } => println!(
            "[{} +{:9.3}ms] STATE id=..{} state={}",
            wall_ms(),
            rel(*qpc),
            &id[id.len().saturating_sub(12)..],
            device::state_name(*state)
        ),
        Event::Added { qpc, id } => println!("[{} +{:9.3}ms] ADDED id=..{}", wall_ms(), rel(*qpc), &id[id.len().saturating_sub(12)..]),
        Event::Removed { qpc, id } => println!("[{} +{:9.3}ms] REMOVED id=..{}", wall_ms(), rel(*qpc), &id[id.len().saturating_sub(12)..]),
        Event::Property { qpc, id, fmtid, pid } => println!(
            "[{} +{:9.3}ms] PROPERTY id=..{} key={{{:?}}},{}",
            wall_ms(),
            rel(*qpc),
            &id[id.len().saturating_sub(12)..],
            fmtid,
            pid
        ),
    }
}

fn cmd_enumerate() -> R<()> {
    let (tx, _rx) = mpsc::channel();
    let m = Monitor::new(tx, false)?;
    for (flow, label) in [(eRender, "RENDER"), (eCapture, "CAPTURE")] {
        println!("== {label} endpoints (all states) ==");
        for d in device::enumerate_all(&m.en, flow)? {
            device::print_device(&d, "  ");
        }
    }
    let def = device::describe(&device::default_render(&m.en)?)?;
    println!("== default render (console) ==");
    device::print_device(&def, "  ");
    Ok(())
}

/// `watch` follows the default device: when the default changes, volume notifications are re-bound.
fn cmd_watch(secs: Option<u64>) -> R<()> {
    let (tx, rx) = mpsc::channel();
    let mut m = Monitor::new(tx, true)?;
    let info = m.bind_default()?;
    println!("watching default render device:");
    device::print_device(&info, "  ");
    println!("(change volume with keys/flyout/mixer, toggle mute, switch default device, connect/disconnect Bluetooth)");
    let t0 = sys::qpc();
    let start = Instant::now();
    let mut last_rebind: Option<Instant> = None;
    loop {
        if let Some(s) = secs {
            if start.elapsed() >= Duration::from_secs(s) {
                break;
            }
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(e) => {
                print_event(&e, t0);
                // Windows emits one OnDefaultDeviceChanged per role (console/multimedia/communications):
                // re-bind once per burst, only for the render/console role.
                if let Event::DefaultChanged { flow: 0, role: 0, id, .. } = &e {
                    if m.bound_id() != Some(id.as_str()) && last_rebind.map_or(true, |t| t.elapsed() > Duration::from_millis(50)) {
                        last_rebind = Some(Instant::now());
                        match m.bind_default() {
                            Ok(i) => {
                                println!("  -> re-bound to new default render device:");
                                device::print_device(&i, "     ");
                            }
                            Err(err) => println!("  -> re-bind failed (no default device?): {err}"),
                        }
                    }
                }
                if let Event::Removed { id, .. } | Event::State { id, .. } = &e {
                    if m.bound_id() == Some(id.as_str()) {
                        if let Event::State { state, .. } = &e {
                            println!("  (bound endpoint state is now {})", device::state_name(*state));
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn cmd_bench(secs: u64) -> R<()> {
    let s0 = sys::proc_stats();
    let (tx, rx) = mpsc::channel();
    let t_reg = Instant::now();
    let mut m = Monitor::new(tx, true)?;
    m.bind_default()?;
    let reg_ms = t_reg.elapsed().as_secs_f64() * 1000.0;
    let s1 = sys::proc_stats();
    let wall = Instant::now();
    let mut events = 0u32;
    while wall.elapsed() < Duration::from_secs(secs) {
        if rx.recv_timeout(Duration::from_millis(500)).is_ok() {
            events += 1;
        }
    }
    let s2 = sys::proc_stats();
    let w = wall.elapsed().as_secs_f64();
    let cpu_ms = (s2.cpu_100ns - s1.cpu_100ns) as f64 / 10_000.0;
    println!("bench: registration (enumerator + notification client + bind default + RegisterControlChangeNotify) = {reg_ms:.2} ms");
    println!("bench: baseline  (pre-COM)      working_set={:.2} MiB private={:.2} MiB", mib(s0.working_set), mib(s0.private_bytes));
    println!("bench: registered (after bind)  working_set={:.2} MiB private={:.2} MiB", mib(s1.working_set), mib(s1.private_bytes));
    println!("bench: after {w:.1}s idle        working_set={:.2} MiB private={:.2} MiB peak_ws={:.2} MiB", mib(s2.working_set), mib(s2.private_bytes), mib(s2.peak_working_set));
    println!("bench: idle CPU = {cpu_ms:.3} ms over {w:.1} s  => {:.5} % of one core; events seen while idle = {events}", cpu_ms / (w * 1000.0) * 100.0);
    Ok(())
}

fn mib(b: usize) -> f64 {
    b as f64 / 1024.0 / 1024.0
}

fn cmd_poke(ctx: &str, what: &str) -> R<()> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::System::Com::CLSCTX_ALL;
    let (tx, _rx) = mpsc::channel();
    let m = Monitor::new(tx, false)?;
    let dev = device::default_render(&m.en)?;
    let vol: IAudioEndpointVolume = unsafe { dev.Activate(CLSCTX_ALL, None)? };
    let ctx_guid = match ctx {
        "null" => GUID::zeroed(),
        _ => GUID::new()?,
    };
    // PID of this process is irrelevant to Windows; only the GUID is carried.
    let t = sys::qpc();
    match what {
        "mute0" => unsafe { vol.SetMute(false, &ctx_guid)? },
        "mute1" => unsafe { vol.SetMute(true, &ctx_guid)? },
        s => unsafe { vol.SetMasterVolumeLevelScalar(s.parse::<f32>()?, &ctx_guid)? },
    }
    println!("POKE qpc={t} ctx={ctx_guid:?}");
    Ok(())
}

fn main() -> R<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("enumerate") => cmd_enumerate(),
        Some("watch") => cmd_watch(args.get(1).and_then(|s| s.parse().ok())),
        Some("bench") => cmd_bench(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60)),
        Some("selftest") => selftest::run(),
        Some("poke") => cmd_poke(args.get(1).map(String::as_str).unwrap_or("new"), args.get(2).map(String::as_str).unwrap_or("0.5")),
        _ => {
            eprintln!("usage: t0008-win-volume enumerate | watch [secs] | bench [secs] | selftest");
            Ok(())
        }
    }
}
