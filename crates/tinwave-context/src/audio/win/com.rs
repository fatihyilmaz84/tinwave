//! The Windows COM layer: the only module of the crate that uses `unsafe` (ADR 0004).
//!
//! # Rules this module keeps
//!
//! - **All COM calls happen on the provider thread**, which initialises COM as MTA
//!   ([`ComGuard`]) and owns every interface pointer here.
//! - **Callbacks make no COM call.** `OnNotify` and the `IMMNotificationClient` methods run on
//!   RPC worker threads. Calling back into MMDevice from inside `IMMNotificationClient` can
//!   deadlock, so they only read the clock, copy their arguments and `try_send` a
//!   [`RawEvent`]. Re-binding, property reads and `Unregister*` happen on the provider thread
//!   when it handles that event.
//! - Every `unsafe` block says why it is sound. They fall into four kinds: a call to a COM
//!   method on an interface this thread owns; a pointer handed to us by Windows for the duration
//!   of a callback (copied, never stored); a `PWSTR` allocated by COM (freed with
//!   `CoTaskMemFree`); and a `PROPVARIANT` we cleared.
//! - Nothing here reads a friendly name or container id, and nothing hooks input.

#![allow(unsafe_code)]

use std::ffi::c_void;

use tinwave_signals::SignalBus;
use tracing::debug;
use windows::core::{implement, Interface, Result as WinResult, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, EDataFlow, ERole, IConnector, IDeviceTopology, IMMDevice,
    IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl, IPart,
    MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, DEVICE_STATE_ACTIVE,
    DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT, DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::Media::KernelStreaming::{IKsJackDescription, KSJACK_DESCRIPTION};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToUInt32};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};

use crate::audio::backend::{
    Backend, BindError, BindTarget, BoundEndpoint, DeviceDescription, DeviceState, Flow, RawEvent,
    Role,
};
use crate::audio::classify::{transport_from_connected_id, JackDescription, Transport};
use crate::audio::runtime::BackendContext;

/// `HRESULT_FROM_WIN32(ERROR_NOT_FOUND)`: what MMDevice returns when there is no such endpoint.
const E_NOTFOUND: HRESULT = HRESULT(0x8007_0490_u32 as i32);

/// `PKEY_AudioEndpoint_FormFactor` = {1da5d803-d492-4edd-8c23-e0c0ffee7f0e}, 0 (`mmdeviceapi.h`).
const PKEY_FORM_FACTOR: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e),
    pid: 0,
};

/// COM initialised as MTA on the current thread; uninitialised on drop.
struct ComGuard;

impl ComGuard {
    fn init() -> Result<Self, String> {
        // SAFETY: initialises COM on the calling thread. Every success, `S_FALSE` (already
        // initialised in the same mode) included, is paired with exactly one `CoUninitialize`
        // in `Drop`, on the same thread, because the guard is only returned on success and
        // is never sent to another thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|e| format!("CoInitializeEx failed: {e}"))?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        // SAFETY: pairs with the successful `CoInitializeEx` in `init`, on the same thread, after
        // every interface owned by the backend has been released (this guard is its last field).
        unsafe { CoUninitialize() };
    }
}

// ---- callbacks: timestamp, copy, send ----

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    events: crate::audio::runtime::EventSender,
    bus: SignalBus,
    generation: u64,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> WinResult<()> {
        let at = self.bus.now();
        // SAFETY: Windows passes a pointer that is valid for the duration of this call (or
        // null, which `as_ref` handles). The data is copied out, nothing is kept.
        let Some(data) = (unsafe { pnotify.as_ref() }) else {
            return Ok(());
        };
        self.events.send(RawEvent::Volume {
            at,
            generation: self.generation,
            context: data.guidEventContext.to_u128(),
            scalar: data.fMasterVolume,
            muted: data.bMuted.as_bool(),
        });
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCallback {
    events: crate::audio::runtime::EventSender,
    bus: SignalBus,
}

/// Copies an endpoint id out of a callback argument; `None` for a null pointer.
fn id_of(id: &PCWSTR) -> Option<String> {
    if id.is_null() {
        return None;
    }
    // SAFETY: Windows passes a NUL-terminated wide string valid for the duration of the
    // callback; it is copied immediately.
    unsafe { id.to_string() }.ok()
}

fn state_of(state: DEVICE_STATE) -> DeviceState {
    if state == DEVICE_STATE_ACTIVE {
        DeviceState::Active
    } else if state == DEVICE_STATE_DISABLED {
        DeviceState::Disabled
    } else if state == DEVICE_STATE_NOTPRESENT {
        DeviceState::NotPresent
    } else if state == DEVICE_STATE_UNPLUGGED {
        DeviceState::Unplugged
    } else {
        DeviceState::Other
    }
}

fn flow_of(flow: EDataFlow) -> Flow {
    if flow == eRender {
        Flow::Render
    } else if flow == windows::Win32::Media::Audio::eCapture {
        Flow::Capture
    } else {
        Flow::Other
    }
}

fn role_of(role: ERole) -> Role {
    use windows::Win32::Media::Audio::{eCommunications, eConsole};
    if role == eConsole {
        Role::Console
    } else if role == eMultimedia {
        Role::Multimedia
    } else if role == eCommunications {
        Role::Communications
    } else {
        Role::Other
    }
}

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, id: &PCWSTR, state: DEVICE_STATE) -> WinResult<()> {
        if let Some(id) = id_of(id) {
            self.events.send(RawEvent::StateChanged {
                at: self.bus.now(),
                id,
                state: state_of(state),
            });
        }
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> WinResult<()> {
        // A new device is not a change of the one we follow; its activation arrives as a
        // state change.
        Ok(())
    }

    fn OnDeviceRemoved(&self, id: &PCWSTR) -> WinResult<()> {
        if let Some(id) = id_of(id) {
            self.events.send(RawEvent::Removed {
                at: self.bus.now(),
                id,
            });
        }
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, id: &PCWSTR) -> WinResult<()> {
        self.events.send(RawEvent::DefaultChanged {
            at: self.bus.now(),
            flow: flow_of(flow),
            role: role_of(role),
            id: id_of(id),
        });
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> WinResult<()> {
        // High-volume noise (ADR 0004): counted, never forwarded.
        self.events.count_property_noise();
        Ok(())
    }
}

// ---- the backend ----

struct Registration {
    volume: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
}

/// MMDevice notifications and the endpoint-volume registration of the bound render endpoint.
///
/// Field order matters: the COM guard is last so every interface is released before COM is
/// uninitialised.
pub(super) struct WindowsBackend {
    context: BackendContext,
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
    registration: Option<Registration>,
    _com: ComGuard,
}

impl WindowsBackend {
    /// Initialises COM (MTA) on this thread and registers for device notifications.
    pub(super) fn new(context: BackendContext) -> Result<Self, String> {
        let com = ComGuard::init()?;
        // SAFETY: COM is initialised on this thread; `MMDeviceEnumerator` is the documented
        // class id of `IMMDeviceEnumerator`.
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|e| format!("could not create the device enumerator: {e}"))?;
        let client: IMMNotificationClient = DeviceCallback {
            events: context.events.clone(),
            bus: context.bus.clone(),
        }
        .into();
        // SAFETY: `enumerator` and `client` are live interfaces owned by this thread.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client) }
            .map_err(|e| format!("could not register for device notifications: {e}"))?;
        Ok(Self {
            context,
            enumerator,
            client,
            registration: None,
            _com: com,
        })
    }

    fn open(&self, target: BindTarget<'_>) -> Result<IMMDevice, BindError> {
        let device = match target {
            // SAFETY: a COM call on the enumerator this thread owns.
            BindTarget::SystemDefault => unsafe {
                self.enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)
            },
            BindTarget::Device(id) => {
                let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
                // SAFETY: `wide` is NUL-terminated and outlives the call.
                unsafe { self.enumerator.GetDevice(PCWSTR(wide.as_ptr())) }
            }
        };
        device.map_err(|e| {
            if e.code() == E_NOTFOUND {
                BindError::NoDevice
            } else {
                BindError::Os(e.to_string())
            }
        })
    }
}

/// Frees a COM-allocated wide string and returns it as a `String`.
fn take_pwstr(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: `p` was allocated by COM for us (`GetId`, `GetDeviceIdConnectedTo`), is
    // NUL-terminated, and is freed exactly once, here.
    let s = unsafe { p.to_string() }.ok();
    // SAFETY: the same COM-allocated pointer, freed once; `p` is not used afterwards.
    unsafe { CoTaskMemFree(Some(p.0.cast::<c_void>().cast_const())) };
    s
}

fn form_factor_of(device: &IMMDevice) -> Option<u32> {
    // SAFETY: COM calls on an interface this thread owns. The PROPVARIANT is read, then cleared.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&PKEY_FORM_FACTOR).ok()?;
        let form_factor = PropVariantToUInt32(&value).ok();
        let _ = PropVariantClear(&mut value);
        form_factor
    }
}

/// Jack information from the adapter side of the endpoint's connector, if the driver exposes it.
fn jack_of(connector: &IConnector) -> Option<JackDescription> {
    // SAFETY: COM calls on interfaces this thread owns. `Activate` writes a new interface
    // pointer to `raw`, which `from_raw` takes ownership of exactly once.
    unsafe {
        let adapter_side = connector.GetConnectedTo().ok();
        for candidate in [adapter_side, Some(connector.clone())]
            .into_iter()
            .flatten()
        {
            let Ok(part) = candidate.cast::<IPart>() else {
                continue;
            };
            let mut raw: *mut c_void = std::ptr::null_mut();
            if part
                .Activate(CLSCTX_ALL.0, &IKsJackDescription::IID, Some(&mut raw))
                .is_err()
                || raw.is_null()
            {
                continue;
            }
            let jacks = IKsJackDescription::from_raw(raw);
            if jacks.GetJackCount().unwrap_or(0) == 0 {
                continue;
            }
            let mut description = KSJACK_DESCRIPTION::default();
            if jacks.GetJackDescription(0, &mut description).is_ok() {
                return Some(JackDescription {
                    connection_type: description.ConnectionType.0,
                    geo_location: description.GeoLocation.0,
                    gen_location: description.GenLocation.0,
                });
            }
        }
        None
    }
}

/// Reads the classifier's inputs. What cannot be read stays unknown.
fn describe(device: &IMMDevice) -> DeviceDescription {
    let form_factor = form_factor_of(device);
    if form_factor.is_none() {
        debug!("endpoint form factor unreadable");
    }
    let mut transport = Transport::Unknown;
    let mut jack = None;
    // SAFETY: COM calls on interfaces this thread owns.
    let topology = unsafe { device.Activate::<IDeviceTopology>(CLSCTX_ALL, None) };
    match topology {
        Ok(topology) => {
            // SAFETY: as above.
            if let Ok(connector) = unsafe { topology.GetConnector(0) } {
                // SAFETY: as above; the returned string is freed by `take_pwstr`.
                let adapter_path = unsafe { connector.GetDeviceIdConnectedTo() }
                    .ok()
                    .and_then(take_pwstr);
                transport = adapter_path
                    .as_deref()
                    .map_or(Transport::Unknown, transport_from_connected_id);
                jack = jack_of(&connector);
            }
        }
        Err(error) => debug!(%error, "endpoint topology unavailable"),
    }
    DeviceDescription {
        form_factor,
        transport,
        jack,
    }
}

impl Backend for WindowsBackend {
    fn bind(
        &mut self,
        target: BindTarget<'_>,
        generation: u64,
    ) -> Result<BoundEndpoint, BindError> {
        let os = |e: windows::core::Error| BindError::Os(e.to_string());
        let device = self.open(target)?;
        // SAFETY: a COM call on the `IMMDevice` this thread just obtained and owns.
        let state = unsafe { device.GetState() }.map_err(os)?;
        if state != DEVICE_STATE_ACTIVE {
            return Err(BindError::NotActive);
        }
        // SAFETY: a COM call on the owned device; the returned string is freed by `take_pwstr`.
        let id = unsafe { device.GetId() }
            .ok()
            .and_then(take_pwstr)
            .ok_or_else(|| BindError::Os("endpoint id unreadable".to_owned()))?;
        let description = describe(&device);
        // SAFETY: a COM call on the owned device; no activation parameters are passed.
        let volume: IAudioEndpointVolume =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(os)?;
        let callback: IAudioEndpointVolumeCallback = VolumeCallback {
            events: self.context.events.clone(),
            bus: self.context.bus.clone(),
            generation,
        }
        .into();
        // SAFETY: both interfaces are live and owned by this thread; the callback only copies
        // and `try_send`s (module docs), so registering cannot deadlock.
        unsafe { volume.RegisterControlChangeNotify(&callback) }.map_err(os)?;
        // Read after registering: a change in between arrives as an event, none is lost.
        // SAFETY: two COM calls on the endpoint-volume interface this thread owns.
        let read = unsafe { (volume.GetMasterVolumeLevelScalar(), volume.GetMute()) };
        let (volume_scalar, muted) = match read {
            (Ok(scalar), Ok(muted)) => (scalar, muted.as_bool()),
            (Err(e), _) | (_, Err(e)) => {
                // Do not leave a registration behind for an endpoint we cannot read.
                // SAFETY: undoes the registration made above with the same interface and
                // callback. A failure leaves nothing more to undo, so it is ignored.
                let _ = unsafe { volume.UnregisterControlChangeNotify(&callback) };
                return Err(os(e));
            }
        };
        self.registration = Some(Registration { volume, callback });
        Ok(BoundEndpoint {
            id,
            description,
            volume_scalar,
            muted,
        })
    }

    fn unbind(&mut self) {
        if let Some(registration) = self.registration.take() {
            // SAFETY: the interface and callback are the ones registered together in `bind`.
            // Failure is expected when the device is already gone and changes nothing for us.
            if let Err(error) = unsafe {
                registration
                    .volume
                    .UnregisterControlChangeNotify(&registration.callback)
            } {
                debug!(%error, "unregistering from a vanished endpoint");
            }
        }
    }

    fn gain_db(&mut self) -> Option<f32> {
        let registration = self.registration.as_ref()?;
        // SAFETY: a COM call on the bound endpoint interface this thread owns.
        unsafe { registration.volume.GetMasterVolumeLevel() }.ok()
    }
}

impl Drop for WindowsBackend {
    fn drop(&mut self) {
        self.unbind();
        // SAFETY: the pair of the registration in `new`; both interfaces are still live (fields
        // drop after this body). Failure at shutdown is not actionable.
        if let Err(error) = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.client)
        } {
            debug!(%error, "unregistering the device notification client");
        }
    }
}

// ---- live tests only: touch the endpoint with an explicit event context ----

/// The default render endpoint's volume control, for the ignored live tests.
#[cfg(test)]
pub(super) struct LiveEndpoint {
    volume: IAudioEndpointVolume,
    _com: ComGuard,
}

#[cfg(test)]
impl LiveEndpoint {
    pub(super) fn default_render() -> Result<Self, String> {
        let com = ComGuard::init()?;
        // SAFETY: COM is initialised on this thread; calls on interfaces it owns.
        let volume = unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| e.to_string())?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(|e| e.to_string())?;
            device
                .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                .map_err(|e| e.to_string())?
        };
        Ok(Self { volume, _com: com })
    }

    /// The slider scalar. A failed read is an error: a caller that restores a value must never
    /// restore a made-up one.
    pub(super) fn scalar(&self) -> Result<f32, String> {
        // SAFETY: COM call on an owned interface.
        unsafe { self.volume.GetMasterVolumeLevelScalar() }.map_err(|e| e.to_string())
    }

    pub(super) fn muted(&self) -> Result<bool, String> {
        // SAFETY: COM call on an owned interface.
        unsafe { self.volume.GetMute() }
            .map(|m| m.as_bool())
            .map_err(|e| e.to_string())
    }

    /// Sets the scalar with `context` as the event context (`0` is `GUID_NULL`).
    pub(super) fn set_scalar(&self, scalar: f32, context: u128) -> Result<(), String> {
        let guid = windows::core::GUID::from_u128(context);
        // SAFETY: COM call on an owned interface; `guid` outlives the call.
        unsafe { self.volume.SetMasterVolumeLevelScalar(scalar, &guid) }.map_err(|e| e.to_string())
    }

    /// Sets mute with `context` as the event context.
    pub(super) fn set_mute(&self, muted: bool, context: u128) -> Result<(), String> {
        let guid = windows::core::GUID::from_u128(context);
        // SAFETY: COM call on an owned interface; `guid` outlives the call.
        unsafe { self.volume.SetMute(muted, &guid) }.map_err(|e| e.to_string())
    }
}
