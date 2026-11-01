//! Programmatic verification. Always restores the original endpoint volume and mute state (Drop guard).

use crate::monitor::{Event, Monitor, TINWAVE_CTX};
use crate::{device, origin, sys, R};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use windows::core::GUID;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::CLSCTX_ALL;

struct Restore {
    vol: IAudioEndpointVolume,
    volume: f32,
    muted: bool,
}
impl Drop for Restore {
    fn drop(&mut self) {
        unsafe {
            let _ = self.vol.SetMute(self.muted, &TINWAVE_CTX);
            let _ = self.vol.SetMasterVolumeLevelScalar(self.volume, &TINWAVE_CTX);
        }
    }
}

fn drain(rx: &Receiver<Event>, wait_ms: u64) -> Vec<Event> {
    let mut v = Vec::new();
    while let Ok(e) = rx.recv_timeout(Duration::from_millis(wait_ms)) {
        v.push(e);
    }
    v
}

fn vol_events(v: &[Event]) -> Vec<(i64, GUID, f32, bool)> {
    v.iter()
        .filter_map(|e| match e {
            Event::Volume { qpc, ctx, volume, muted, .. } => Some((*qpc, *ctx, *volume, *muted)),
            _ => None,
        })
        .collect()
}

fn check(name: &str, ok: bool, detail: String) -> bool {
    println!("  [{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn run_child(ctx: &str, what: &str) -> R<i64> {
    let exe = std::env::current_exe()?;
    let out = Command::new(exe).args(["poke", ctx, what]).output()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let q = s
        .split_whitespace()
        .find_map(|t| t.strip_prefix("qpc=").and_then(|n| n.parse::<i64>().ok()))
        .ok_or_else(|| format!("child gave no qpc: stdout={s} stderr={}", String::from_utf8_lossy(&out.stderr)))?;
    Ok(q)
}

pub fn run() -> R<()> {
    let (tx, rx) = mpsc::channel();
    let mut m = Monitor::new(tx, true)?;
    let info = m.bind_default()?;
    println!("selftest on default render endpoint: {} (class={})", info.name, info.class.as_str());
    let vol = m.volume().unwrap().clone();
    let (orig_v, orig_m) = unsafe { (vol.GetMasterVolumeLevelScalar()?, vol.GetMute()?.as_bool()) };
    let (mut step, mut nsteps) = (0u32, 0u32);
    unsafe { vol.GetVolumeStepInfo(&mut step, &mut nsteps)? };
    let (mut dmin, mut dmax, mut dinc) = (0f32, 0f32, 0f32);
    let db = unsafe {
        vol.GetVolumeRange(&mut dmin, &mut dmax, &mut dinc)?;
        vol.GetMasterVolumeLevel()?
    };
    println!("original: volume={orig_v:.3} ({db:.2} dB; range {dmin:.2}..{dmax:.2} dB, inc {dinc:.3}) muted={orig_m} step={step}/{nsteps}  (will be restored)");
    let _guard = Restore { vol: vol.clone(), volume: orig_v, muted: orig_m };
    // A small, always-valid target: move by 2 hardware steps, away from the nearer rail.
    let d = (2.0 / (nsteps.max(2) - 1) as f32).max(0.02);
    let target = if orig_v + d <= 1.0 { orig_v + d } else { orig_v - d };
    let target2 = if orig_v + 2.0 * d <= 1.0 { orig_v + 2.0 * d } else { orig_v - 2.0 * d };
    let mut all_ok = true;
    let _ = drain(&rx, 100); // flush registration noise

    // A. our own change, our context GUID -> must be classified "self"
    println!("A. in-process change with Tinwave event-context GUID");
    let t = sys::qpc();
    unsafe { vol.SetMasterVolumeLevelScalar(target, &TINWAVE_CTX)? };
    let ev = vol_events(&drain(&rx, 300));
    all_ok &= check(
        "self classified",
        ev.len() == 1 && ev[0].1 == TINWAVE_CTX,
        format!("events={} origin={:?} latency={:.3} ms vol={:.3}", ev.len(), ev.first().map(|e| origin(&e.1)), ev.first().map_or(f64::NAN, |e| sys::qpc_to_ms(e.0 - t)), ev.first().map_or(f32::NAN, |e| e.2)),
    );

    // B. same process, foreign GUID -> external
    println!("B. in-process change with a foreign event-context GUID");
    let foreign = GUID::new()?;
    let t = sys::qpc();
    unsafe { vol.SetMasterVolumeLevelScalar(target2, &foreign)? };
    let ev = vol_events(&drain(&rx, 300));
    all_ok &= check(
        "foreign classified external",
        ev.len() == 1 && ev[0].1 == foreign && ev[0].1 != TINWAVE_CTX,
        format!("events={} origin={:?} latency={:.3} ms", ev.len(), ev.first().map(|e| origin(&e.1)), ev.first().map_or(f64::NAN, |e| sys::qpc_to_ms(e.0 - t))),
    );

    // C. null context (what plain callers pass; keyboard path is expected to be external too)
    println!("C. in-process change with NULL event context");
    let t = sys::qpc();
    unsafe { vol.SetMasterVolumeLevelScalar(orig_v, std::ptr::null())? };
    let ev = vol_events(&drain(&rx, 300));
    all_ok &= check(
        "null ctx classified external",
        ev.len() == 1 && ev[0].1 == GUID::zeroed(),
        format!("events={} origin={:?} latency={:.3} ms", ev.len(), ev.first().map(|e| origin(&e.1)), ev.first().map_or(f64::NAN, |e| sys::qpc_to_ms(e.0 - t))),
    );

    // D. separate process (like a hardware-key handler, SndVol or another app): latency from the child's
    //    own pre-call QPC (system-wide clock) to our callback.
    println!("D. cross-process change (child process, its own random GUID), 5 trials");
    let mut lat = Vec::new();
    for i in 0..5 {
        let want = if i % 2 == 0 { target } else { orig_v };
        let t_child = run_child("new", &format!("{want}"))?;
        let ev = vol_events(&drain(&rx, 300));
        if let Some(e) = ev.first() {
            let external = e.1 != TINWAVE_CTX;
            lat.push(sys::qpc_to_ms(e.0 - t_child));
            all_ok &= check(&format!("trial {i} external"), external && ev.len() == 1, format!("events={} latency(child pre-call -> callback)={:.3} ms", ev.len(), lat.last().unwrap()));
        } else {
            all_ok &= check(&format!("trial {i}"), false, "no event".into());
        }
    }
    if !lat.is_empty() {
        lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("  cross-process latency ms: min={:.3} median={:.3} max={:.3}", lat[0], lat[lat.len() / 2], lat[lat.len() - 1]);
    }

    // E. mute
    println!("E. mute toggle (cross-process, null ctx) -> MuteToggled path");
    let _ = run_child("null", "mute1")?;
    let ev = vol_events(&drain(&rx, 300));
    all_ok &= check("mute on seen", ev.len() == 1 && ev[0].3, format!("events={} muted={:?} origin={:?}", ev.len(), ev.first().map(|e| e.3), ev.first().map(|e| origin(&e.1))));
    let _ = run_child("null", "mute0")?;
    let ev = vol_events(&drain(&rx, 300));
    all_ok &= check("mute off seen", ev.len() == 1 && !ev[0].3, format!("events={} muted={:?}", ev.len(), ev.first().map(|e| e.3)));
    // NOTE: mute and volume share one notification struct; consumer must diff against its last-known state
    // to emit MuteToggled vs SystemVolumeChanged (a mute-only change repeats the same volume).

    // F. burst: simulated slider drag, 60 changes at ~2 ms spacing
    println!("F. burst of 60 volume sets (slider drag emulation) -> delivery / coalescing");
    let n_hw = nsteps.max(2) as usize;
    let mut expected_distinct = 0usize;
    let mut prev = orig_v;
    let t_burst = sys::qpc();
    for i in 0..60 {
        let k = (i % 8) as f32;
        let v = (orig_v - 0.02 * 4.0 + 0.02 * k).clamp(0.0, 1.0);
        if (v - prev).abs() > f32::EPSILON {
            expected_distinct += 1;
        }
        prev = v;
        unsafe { vol.SetMasterVolumeLevelScalar(v, &foreign)? };
        std::thread::sleep(Duration::from_millis(2));
    }
    let ev = vol_events(&drain(&rx, 400));
    let in_order_last = ev.last().map(|e| e.2);
    println!(
        "  sets={} distinct_value_changes~{} (hardware steps={}) events_received={} burst span={:.1} ms last_event_vol={:?}",
        60,
        expected_distinct,
        n_hw,
        ev.len(),
        sys::qpc_to_ms(ev.last().map_or(0, |e| e.0) - t_burst),
        in_order_last
    );
    all_ok &= check("burst delivered (>=90% of distinct changes, none dropped silently)", ev.len() as f64 >= 0.9 * expected_distinct as f64, format!("{}/{}", ev.len(), expected_distinct));
    unsafe { vol.SetMasterVolumeLevelScalar(orig_v, &TINWAVE_CTX)? };
    let _ = drain(&rx, 200);

    // G. per-app session volume (ISimpleAudioVolume) must NOT fire the endpoint callback: Tinwave's own
    //    in-app volume slider never aliases into SystemVolumeChanged.
    println!("G. per-app session volume (ISimpleAudioVolume) must not produce endpoint events");
    all_ok &= session_volume_test(&m, &rx)?;

    // H. restore and verify
    drop(_guard);
    let _ = drain(&rx, 200);
    let (fv, fm) = unsafe { (vol.GetMasterVolumeLevelScalar()?, vol.GetMute()?.as_bool()) };
    all_ok &= check("original volume/mute restored", (fv - orig_v).abs() < 0.005 && fm == orig_m, format!("now volume={fv:.3} muted={fm} (was {orig_v:.3}/{orig_m})"));
    println!("{}", if all_ok { "SELFTEST: ALL PASS" } else { "SELFTEST: FAILURES (see above)" });
    Ok(())
}

fn session_volume_test(m: &Monitor, rx: &Receiver<Event>) -> R<bool> {
    let dev = device::default_render(&m.en)?;
    let ac: IAudioClient = unsafe { dev.Activate(CLSCTX_ALL, None)? };
    unsafe {
        let fmt = ac.GetMixFormat()?;
        ac.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 10_000_000, 0, fmt, None)?;
        windows::Win32::System::Com::CoTaskMemFree(Some(fmt as *const _));
        let sv: ISimpleAudioVolume = ac.GetService()?;
        let before = sv.GetMasterVolume()?;
        let _ = drain(rx, 100);
        sv.SetMasterVolume(0.5, &TINWAVE_CTX)?;
        sv.SetMute(true, &TINWAVE_CTX)?;
        sv.SetMute(false, &TINWAVE_CTX)?;
        let ev = vol_events(&drain(rx, 400));
        sv.SetMasterVolume(before, &TINWAVE_CTX)?; // restore (Windows remembers per-app volume)
        let ok = ev.is_empty();
        Ok(check("session volume/mute changes invisible to endpoint callback", ok, format!("endpoint events={} (expected 0); session volume was {before:.2}, restored", ev.len())))
    }
}
