//! Output backends: (A) cpal 0.18 (WASAPI shared under the hood), (B) direct WASAPI via the `wasapi` crate
//! (shared + exclusive, event driven, IMMNotificationClient for default-device changes).
use crate::engine::{Backend, OutSpec, Renderer, CH};
use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering::*};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------- device listing

pub struct DevInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub mix: String,
}

pub fn list_wasapi_devices() -> Result<Vec<DevInfo>> {
    use wasapi::*;
    let _ = initialize_mta();
    let en = DeviceEnumerator::new().map_err(|e| anyhow!("{e}"))?;
    let def = en.get_default_device(&Direction::Render).ok().and_then(|d| d.get_id().ok());
    let coll = en.get_device_collection(&Direction::Render).map_err(|e| anyhow!("{e}"))?;
    let mut v = vec![];
    for d in &coll {
        let d = d.map_err(|e| anyhow!("{e}"))?;
        let id = d.get_id().map_err(|e| anyhow!("{e}"))?;
        let name = d.get_friendlyname().unwrap_or_default();
        let mix = d
            .get_iaudioclient()
            .and_then(|c| c.get_mixformat())
            .map(|f| format!("{} Hz / {} ch / {} bit", f.get_samplespersec(), f.get_nchannels(), f.get_bitspersample()))
            .unwrap_or_else(|e| format!("? ({e})"));
        v.push(DevInfo { is_default: def.as_deref() == Some(id.as_str()), id, name, mix });
    }
    Ok(v)
}

/// Watches default-render-device / device-state changes (IMMNotificationClient) on its own thread.
pub struct DefaultWatcher {
    pub changed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    th: Option<std::thread::JoinHandle<()>>,
}

impl DefaultWatcher {
    pub fn start(log: bool) -> Result<Self> {
        let changed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (c2, s2) = (changed.clone(), stop.clone());
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let th = std::thread::spawn(move || {
            use wasapi::*;
            let _ = initialize_mta();
            let res = (|| -> Result<_, String> {
                let en = DeviceEnumerator::new().map_err(|e| e.to_string())?;
                let mut cb = DeviceEventCallbacks::new();
                let c = c2.clone();
                cb.set_default_device_callback(move |dir, role, id| {
                    if log {
                        println!("  [notify] default {dir} device (role {role}) -> {:?}", id);
                    }
                    c.store(true, SeqCst);
                });
                let c = c2.clone();
                cb.set_device_state_callback(move |id, st| {
                    if log {
                        println!("  [notify] device {id} state {st}");
                    }
                    c.store(true, SeqCst);
                });
                let reg = en.register_notification_callback(cb).map_err(|e| e.to_string())?;
                Ok((en, reg))
            })();
            match res {
                Ok(keep) => {
                    let _ = tx.send(Ok(()));
                    while !s2.load(Relaxed) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    drop(keep);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });
        rx.recv().map_err(|e| anyhow!("{e}"))?.map_err(|e| anyhow!("{e}"))?;
        Ok(Self { changed, stop, th: Some(th) })
    }
}
impl Drop for DefaultWatcher {
    fn drop(&mut self) {
        self.stop.store(true, SeqCst);
        if let Some(t) = self.th.take() {
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------- A: cpal

pub struct CpalBackend {
    _stream: cpal::Stream,
    name: String,
    rate: u32,
    lost: Arc<AtomicBool>,
}

impl Backend for CpalBackend {
    fn name(&self) -> String {
        format!("cpal:{}", self.name)
    }
    fn spec(&self) -> OutSpec {
        OutSpec { rate: self.rate }
    }
    fn lost(&self) -> bool {
        self.lost.load(Relaxed)
    }
}

pub fn list_cpal_devices() -> Result<()> {
    let host = cpal::default_host();
    let def = host.default_output_device().and_then(|d| d.id().ok()).map(|i| i.to_string());
    for d in host.output_devices()? {
        let id = d.id().map(|i| i.to_string()).unwrap_or_default();
        let name = d.description().map(|x| x.name().to_string()).unwrap_or_default();
        let cfg = d.default_output_config().map(|c| format!("{:?}", c)).unwrap_or_else(|e| format!("{e}"));
        println!("  cpal {} {name}  id={id}\n      default cfg: {cfg}", if def.as_deref() == Some(id.as_str()) { "*" } else { " " });
    }
    Ok(())
}

pub fn open_cpal(device: Option<&str>, rate: u32, mut renderer: Renderer) -> Result<Box<dyn Backend>> {
    let host = cpal::default_host();
    let dev = match device {
        None => host.default_output_device().ok_or_else(|| anyhow!("no default output device"))?,
        Some(sel) => {
            let mut found = None;
            for d in host.output_devices()? {
                let id = d.id().map(|i| i.to_string()).unwrap_or_default();
                let name = d.description().map(|x| x.name().to_string()).unwrap_or_default();
                if id == sel || name.to_lowercase().contains(&sel.to_lowercase()) {
                    found = Some(d);
                    break;
                }
            }
            found.ok_or_else(|| anyhow!("device '{sel}' not found"))?
        }
    };
    let name = dev.description().map(|x| x.name().to_string()).unwrap_or_default();
    let cfg = cpal::StreamConfig { channels: CH as u16, sample_rate: rate, buffer_size: cpal::BufferSize::Default };
    let lost = Arc::new(AtomicBool::new(false));
    let l2 = lost.clone();
    let stream = dev.build_output_stream(
        cfg,
        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
            let ts = info.timestamp();
            let lat = ts.playback.checked_duration_since(ts.callback).unwrap_or_default();
            let lat_frames = (lat.as_secs_f64() * rate as f64) as u64;
            renderer.render(data, lat_frames);
        },
        move |e| {
            eprintln!("  [cpal error] {e}");
            use cpal::ErrorKind::*;
            if matches!(e.kind(), DeviceNotAvailable | StreamInvalidated | DeviceChanged) {
                l2.store(true, SeqCst);
            }
        },
        None,
    )?;
    stream.play()?;
    Ok(Box::new(CpalBackend { _stream: stream, name, rate, lost }))
}

// ---------------------------------------------------------------- B: direct WASAPI

pub struct WasapiBackend {
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    th: Option<std::thread::JoinHandle<()>>,
    name: String,
    rate: u32,
    mode: String,
}

impl Backend for WasapiBackend {
    fn name(&self) -> String {
        format!("wasapi:{}", self.name)
    }
    fn spec(&self) -> OutSpec {
        OutSpec { rate: self.rate }
    }
    fn lost(&self) -> bool {
        self.lost.load(Relaxed)
    }
    fn mode(&self) -> String {
        self.mode.clone()
    }
}
impl Drop for WasapiBackend {
    fn drop(&mut self) {
        self.stop.store(true, SeqCst);
        if let Some(t) = self.th.take() {
            let _ = t.join();
        }
    }
}

fn mmcss() {
    use windows::core::w;
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    let mut idx = 0u32;
    unsafe {
        let _ = AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut idx);
    }
}

pub fn open_wasapi_shared(device: Option<&str>, rate: u32, renderer: Renderer) -> Result<Box<dyn Backend>> {
    open_wasapi(device, rate, renderer, false)
}
pub fn open_wasapi_exclusive(device: Option<&str>, rate: u32, renderer: Renderer) -> Result<Box<dyn Backend>> {
    open_wasapi(device, rate, renderer, true)
}

fn open_wasapi(device: Option<&str>, rate: u32, mut renderer: Renderer, exclusive: bool) -> Result<Box<dyn Backend>> {
    let stop = Arc::new(AtomicBool::new(false));
    let lost = Arc::new(AtomicBool::new(false));
    let (s2, l2) = (stop.clone(), lost.clone());
    let sel = device.map(|s| s.to_string());
    let (tx, rx) = std::sync::mpsc::channel::<Result<(String, String), String>>();
    let th = std::thread::Builder::new().name("wasapi-render".into()).spawn(move || {
        use wasapi::*;
        let _ = initialize_mta();
        mmcss();
        let setup = || -> Result<_, String> {
            let en = DeviceEnumerator::new().map_err(|e| e.to_string())?;
            let dev = match &sel {
                None => en.get_default_device(&Direction::Render).map_err(|e| e.to_string())?,
                Some(s) => {
                    let coll = en.get_device_collection(&Direction::Render).map_err(|e| e.to_string())?;
                    let mut found = None;
                    for d in &coll {
                        let d = d.map_err(|e| e.to_string())?;
                        let id = d.get_id().unwrap_or_default();
                        let nm = d.get_friendlyname().unwrap_or_default();
                        if id == *s || nm.to_lowercase().contains(&s.to_lowercase()) {
                            found = Some(d);
                            break;
                        }
                    }
                    found.ok_or_else(|| format!("device '{s}' not found"))?
                }
            };
            let name = dev.get_friendlyname().unwrap_or_default();
            let mut client = dev.get_iaudioclient().map_err(|e| e.to_string())?;
            let (def, min) = client.get_device_period().map_err(|e| e.to_string())?;
            let (fmt, mode, mode_s) = if exclusive {
                // Ask the driver which integer format it takes at this rate.
                let want = WaveFormat::new(24, 24, &SampleType::Int, rate as usize, 2, None);
                let f = client.is_supported_exclusive_with_quirks(&want).map_err(|e| format!("exclusive format: {e}"))?;
                let period = client
                    .calculate_aligned_period_near(3 * min / 2, Some(128), &f)
                    .map_err(|e| format!("period: {e}"))?;
                (f.clone(), StreamMode::EventsExclusive { period_hns: period }, format!("exclusive {}b/{}vb period {} us", f.get_bitspersample(), f.get_validbitspersample(), period / 10))
            } else {
                let f = WaveFormat::new(32, 32, &SampleType::Float, rate as usize, 2, None);
                (f, StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: def }, format!("shared f32 period {} us", def / 10))
            };
            client.initialize_client(&fmt, &Direction::Render, &mode).map_err(|e| format!("initialize: {e}"))?;
            let ev = client.set_get_eventhandle().map_err(|e| e.to_string())?;
            let rc = client.get_audiorenderclient().map_err(|e| e.to_string())?;
            Ok((en, name, client, fmt, ev, rc, mode_s))
        };
        let (_en, name, client, fmt, ev, rc, mode_s) = match setup() {
            Ok(x) => x,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let _ = tx.send(Ok((name, mode_s)));

        let bpf = fmt.get_blockalign() as usize;
        let bits = fmt.get_bitspersample();
        let is_float = bits == 32 && matches!(fmt.get_subformat(), Ok(SampleType::Float));
        let buf_frames = client.get_buffer_size().unwrap_or(4096) as usize;
        let mut scratch = vec![0.0f32; buf_frames * CH];
        let mut bytes = vec![0u8; buf_frames * bpf];
        let stream_lat_hns = unsafe { client.as_iaudioclient().GetStreamLatency().unwrap_or(0) };
        let stream_lat_frames = (stream_lat_hns as u64) * rate as u64 / 10_000_000;

        let mut fill = |renderer: &mut Renderer, frames: usize, padding: usize| -> bool {
            let out = &mut scratch[..frames * CH];
            renderer.render(out, (padding as u64) + stream_lat_frames);
            let dst = &mut bytes[..frames * bpf];
            if is_float {
                for (i, s) in out.iter().enumerate() {
                    dst[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
                }
            } else if bits == 16 {
                for (i, s) in out.iter().enumerate() {
                    let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                    dst[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
                }
            } else {
                // 24-in-32 or 32-bit int, MSB-aligned.
                for (i, s) in out.iter().enumerate() {
                    let v = (s.clamp(-1.0, 1.0) as f64 * 2147483647.0) as i32;
                    dst[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            rc.write_to_device(frames, dst, None).is_ok()
        };

        // Prime the buffer with real data (silence flags are not used: first callback may carry audio).
        let avail = client.get_available_space_in_frames().unwrap_or(0) as usize;
        if !fill(&mut renderer, avail, 0) {
            l2.store(true, SeqCst);
            return;
        }
        if client.start_stream().is_err() {
            l2.store(true, SeqCst);
            return;
        }
        while !s2.load(Relaxed) {
            if ev.wait_for_event(200).is_err() {
                // timeout: also catches an invalidated device (no more events)
                if client.get_current_padding().is_err() {
                    l2.store(true, SeqCst);
                    break;
                }
                continue;
            }
            let avail = match client.get_available_space_in_frames() {
                Ok(a) => a as usize,
                Err(_) => {
                    l2.store(true, SeqCst);
                    break;
                }
            };
            let padding = buf_frames.saturating_sub(avail);
            if avail > 0 && !fill(&mut renderer, avail.min(buf_frames), padding) {
                l2.store(true, SeqCst);
                break;
            }
        }
        let _ = client.stop_stream();
    })?;
    let (name, mode) = rx.recv().map_err(|e| anyhow!("{e}"))?.map_err(|e| anyhow!("{e}"))?;
    if name.is_empty() {
        bail!("no device name");
    }
    Ok(Box::new(WasapiBackend { stop, lost, th: Some(th), name, rate, mode }))
}
