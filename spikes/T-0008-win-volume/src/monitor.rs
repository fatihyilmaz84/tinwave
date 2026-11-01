//! Notification plumbing: `IAudioEndpointVolumeCallback` + `IMMNotificationClient`.
//!
//! Callbacks run on COM/RPC worker threads. They do the absolute minimum (timestamp + channel send);
//! all COM calls (re-binding to a new default device, property reads) happen on the owner thread,
//! because calling back into MMDevice API from inside an IMMNotificationClient method can deadlock.

use crate::device::{self, pcwstr_to_string};
use crate::sys::qpc;
use std::sync::mpsc::Sender;
use windows::core::{implement, Result, GUID, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::CLSCTX_ALL;

/// Event-context GUID Tinwave will pass whenever it changes *endpoint* volume/mute itself.
/// (Prototype constant; production would generate one per process start or hard-code one.)
pub const TINWAVE_CTX: GUID = GUID::from_u128(0x7a1c0de0_0008_4a5e_b00c_71177a7e0001);

#[derive(Debug, Clone)]
pub enum Event {
    Volume { qpc: i64, ctx: GUID, volume: f32, muted: bool, channels: u32, endpoint: String },
    DefaultChanged { qpc: i64, flow: i32, role: i32, id: String },
    State { qpc: i64, id: String, state: u32 },
    Added { qpc: i64, id: String },
    Removed { qpc: i64, id: String },
    Property { qpc: i64, id: String, fmtid: GUID, pid: u32 },
}

#[implement(IAudioEndpointVolumeCallback)]
pub struct VolCb {
    tx: Sender<Event>,
    endpoint: String,
}

impl IAudioEndpointVolumeCallback_Impl for VolCb_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        let t = qpc();
        if pnotify.is_null() {
            return Ok(());
        }
        let d = unsafe { &*pnotify };
        let _ = self.tx.send(Event::Volume {
            qpc: t,
            ctx: d.guidEventContext,
            volume: d.fMasterVolume,
            muted: d.bMuted.as_bool(),
            channels: d.nChannels,
            endpoint: self.endpoint.clone(),
        });
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
pub struct NotifCb {
    tx: Sender<Event>,
}

impl IMMNotificationClient_Impl for NotifCb_Impl {
    fn OnDeviceStateChanged(&self, id: &PCWSTR, state: DEVICE_STATE) -> Result<()> {
        let _ = self.tx.send(Event::State { qpc: qpc(), id: pcwstr_to_string(id), state: state.0 });
        Ok(())
    }
    fn OnDeviceAdded(&self, id: &PCWSTR) -> Result<()> {
        let _ = self.tx.send(Event::Added { qpc: qpc(), id: pcwstr_to_string(id) });
        Ok(())
    }
    fn OnDeviceRemoved(&self, id: &PCWSTR) -> Result<()> {
        let _ = self.tx.send(Event::Removed { qpc: qpc(), id: pcwstr_to_string(id) });
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, id: &PCWSTR) -> Result<()> {
        let _ = self.tx.send(Event::DefaultChanged { qpc: qpc(), flow: flow.0, role: role.0, id: pcwstr_to_string(id) });
        Ok(())
    }
    fn OnPropertyValueChanged(&self, id: &PCWSTR, key: &PROPERTYKEY) -> Result<()> {
        let _ = self.tx.send(Event::Property { qpc: qpc(), id: pcwstr_to_string(id), fmtid: key.fmtid, pid: key.pid });
        Ok(())
    }
}

struct Bound {
    id: String,
    vol: IAudioEndpointVolume,
    cb: IAudioEndpointVolumeCallback,
}

pub struct Monitor {
    pub en: IMMDeviceEnumerator,
    nc: IMMNotificationClient,
    tx: Sender<Event>,
    bound: Option<Bound>,
}

impl Monitor {
    /// Registers the device-level notification client (always) and binds endpoint-volume notifications
    /// to the current default render device (if `bind_default`).
    pub fn new(tx: Sender<Event>, with_device_events: bool) -> Result<Self> {
        let en: IMMDeviceEnumerator = unsafe {
            windows::Win32::System::Com::CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?
        };
        let nc: IMMNotificationClient = NotifCb { tx: tx.clone() }.into();
        if with_device_events {
            unsafe { en.RegisterEndpointNotificationCallback(&nc)? };
        }
        Ok(Self { en, nc, tx, bound: None })
    }

    pub fn bound_id(&self) -> Option<&str> {
        self.bound.as_ref().map(|b| b.id.as_str())
    }

    pub fn bind_default(&mut self) -> Result<device::DeviceInfo> {
        let dev = device::default_render(&self.en)?;
        let info = device::describe(&dev)?;
        self.unbind();
        let vol: IAudioEndpointVolume = unsafe { dev.Activate(CLSCTX_ALL, None)? };
        let cb: IAudioEndpointVolumeCallback = VolCb { tx: self.tx.clone(), endpoint: info.id.clone() }.into();
        unsafe { vol.RegisterControlChangeNotify(&cb)? };
        self.bound = Some(Bound { id: info.id.clone(), vol, cb });
        Ok(info)
    }

    pub fn unbind(&mut self) {
        if let Some(b) = self.bound.take() {
            // Unregister can fail if the device has already gone away; that is not an error for us.
            let _ = unsafe { b.vol.UnregisterControlChangeNotify(&b.cb) };
        }
    }

    pub fn volume(&self) -> Option<&IAudioEndpointVolume> {
        self.bound.as_ref().map(|b| &b.vol)
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.unbind();
        let _ = unsafe { self.en.UnregisterEndpointNotificationCallback(&self.nc) };
    }
}
