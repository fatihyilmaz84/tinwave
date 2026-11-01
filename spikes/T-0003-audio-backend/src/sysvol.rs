//! Proves the `SystemVolumeChanged` hook: IAudioEndpointVolume::RegisterControlChangeNotify.
//! Nudges the master volume by 1 % and restores it, measuring notification latency.
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::core::implement;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

#[implement(IAudioEndpointVolumeCallback)]
struct Cb(Arc<Mutex<Vec<(Instant, f32, bool)>>>);

impl IAudioEndpointVolumeCallback_Impl for Cb_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        let d = unsafe { &*pnotify };
        self.0.lock().unwrap().push((Instant::now(), d.fMasterVolume, d.bMuted.as_bool()));
        Ok(())
    }
}

pub fn run(extra_secs: u64) -> Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None)?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let cb: IAudioEndpointVolumeCallback = Cb(events.clone()).into();
        vol.RegisterControlChangeNotify(&cb)?;
        let orig = vol.GetMasterVolumeLevelScalar()?;
        println!("endpoint master volume = {orig:.3}; nudging by 1 % and restoring");
        let t0 = Instant::now();
        let nudged = if orig >= 0.011 { orig - 0.01 } else { orig + 0.01 };
        vol.SetMasterVolumeLevelScalar(nudged, std::ptr::null())?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        vol.SetMasterVolumeLevelScalar(orig, std::ptr::null())?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        if extra_secs > 0 {
            println!("listening {extra_secs} s more: press the keyboard volume keys / mute now");
            std::thread::sleep(std::time::Duration::from_secs(extra_secs));
        }
        vol.UnregisterControlChangeNotify(&cb)?;
        let ev = events.lock().unwrap();
        println!("received {} notifications:", ev.len());
        for (t, v, m) in ev.iter() {
            println!("   +{:.1} ms: master volume {v:.3} muted {m}", t.duration_since(t0).as_secs_f64() * 1000.0);
        }
        println!("restored to {:.3}", vol.GetMasterVolumeLevelScalar()?);
    }
    Ok(())
}
