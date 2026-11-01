//! Endpoint enumeration + property reads (form factor, transport via topology).

use crate::classify::{self, Class, Transport};
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc, PropVariantToUInt32};
use windows::Win32::System::Com::*;

/// PKEY_AudioEndpoint_FormFactor {1da5d803-d492-4edd-8c23-e0c0ffee7f0e}, 0 (mmdeviceapi.h)
pub const PKEY_FORM_FACTOR: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e),
    pid: 0,
};
/// PKEY_Device_ContainerId {8c7ed206-3f8a-4827-b3ab-ae9e1faefc6c}, 2 : groups all endpoints of one physical product
pub const PKEY_CONTAINER_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x8c7ed206_3f8a_4827_b3ab_ae9e1faefc6c),
    pid: 2,
};

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub state: u32,
    pub form_factor: Option<u32>,
    pub connected_to: String,
    pub connector_type: Option<i32>,
    /// (ConnectionType, GeoLocation, GenLocation, IsConnected) from IKsJackDescription, if exposed
    pub jack: Option<(i32, i32, i32, bool)>,
    pub container: String,
    pub transport: Transport,
    pub class: Class,
}

pub fn state_name(s: u32) -> &'static str {
    match s {
        1 => "ACTIVE",
        2 => "DISABLED",
        4 => "NOTPRESENT",
        8 => "UNPLUGGED",
        _ => "?",
    }
}

unsafe fn pwstr_to_string_free(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    s
}

pub fn pcwstr_to_string(p: &PCWSTR) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { p.to_string().unwrap_or_default() }
    }
}

pub fn describe(dev: &IMMDevice) -> windows::core::Result<DeviceInfo> {
    unsafe {
        let id = pwstr_to_string_free(dev.GetId()?);
        let state = dev.GetState()?.0;
        let store = dev.OpenPropertyStore(STGM_READ)?;

        let mut name = String::new();
        if let Ok(mut pv) = store.GetValue(&PKEY_Device_FriendlyName) {
            if let Ok(p) = PropVariantToStringAlloc(&pv) {
                name = pwstr_to_string_free(p);
            }
            let _ = PropVariantClear(&mut pv);
        }
        let mut form_factor = None;
        if let Ok(mut pv) = store.GetValue(&PKEY_FORM_FACTOR) {
            form_factor = PropVariantToUInt32(&pv).ok();
            let _ = PropVariantClear(&mut pv);
        }
        let mut container = String::new();
        if let Ok(mut pv) = store.GetValue(&PKEY_CONTAINER_ID) {
            if let Ok(p) = PropVariantToStringAlloc(&pv) {
                container = pwstr_to_string_free(p);
            }
            let _ = PropVariantClear(&mut pv);
        }

        // Topology: endpoint connector -> id of the adapter device it is wired to.
        let mut connected_to = String::new();
        let mut connector_type = None;
        let mut jack = None;
        if let Ok(topo) = dev.Activate::<IDeviceTopology>(CLSCTX_ALL, None) {
            if let Ok(conn) = topo.GetConnector(0) {
                if let Ok(p) = conn.GetDeviceIdConnectedTo() {
                    connected_to = pwstr_to_string_free(p);
                }
                connector_type = conn.GetType().ok().map(|t| t.0);
                jack = jack_of(&conn);
            }
        }
        let transport = classify::transport_from_connected_id(&connected_to);
        // "internal" = laptop speakers etc.: GenLocation Internal, or ATAPI/inside-lid geo.
        let internal = jack.is_some_and(|(conn, geo, gen, _)| gen == 1 || conn == 3 || geo == 9);
        let class = classify::classify(form_factor, transport, internal);
        Ok(DeviceInfo { id, name, state, form_factor, connected_to, connector_type, jack, container, transport, class })
    }
}

pub fn print_device(d: &DeviceInfo, indent: &str) {
    println!(
        "{indent}{name}\n{indent}  state={state} form_factor={ff} transport={tr} => class={class}\n{indent}  connector_type={ct:?} jack(conn,geo,gen,connected)={jack:?} container={cont}\n{indent}  connected_to={ct_to}\n{indent}  id={id}",
        name = d.name,
        state = state_name(d.state),
        ff = d.form_factor.map(classify::form_factor_name).unwrap_or("-"),
        tr = d.transport.as_str(),
        class = d.class.as_str(),
        ct = d.connector_type,
        jack = d.jack,
        cont = d.container,
        ct_to = d.connected_to,
        id = d.id,
    );
}

pub fn enumerate_all(en: &IMMDeviceEnumerator, flow: EDataFlow) -> windows::core::Result<Vec<DeviceInfo>> {
    let mut out = Vec::new();
    unsafe {
        let coll = en.EnumAudioEndpoints(flow, DEVICE_STATE(DEVICE_STATEMASK_ALL))?;
        for i in 0..coll.GetCount()? {
            let dev = coll.Item(i)?;
            out.push(describe(&dev)?);
        }
    }
    Ok(out)
}

pub fn default_render(en: &IMMDeviceEnumerator) -> windows::core::Result<IMMDevice> {
    unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole) }
}

/// Best-effort IKsJackDescription lookup on the adapter-side connector.
fn jack_of(conn: &IConnector) -> Option<(i32, i32, i32, bool)> {
    use windows::core::Interface;
    use windows::Win32::Media::KernelStreaming::{IKsJackDescription, KSJACK_DESCRIPTION};
    unsafe {
        for c in [conn.GetConnectedTo().ok(), Some(conn.clone())].into_iter().flatten() {
            let Ok(part) = c.cast::<IPart>() else { continue };
            let mut raw: *mut core::ffi::c_void = std::ptr::null_mut();
            if part.Activate(CLSCTX_ALL.0, &IKsJackDescription::IID, Some(&mut raw)).is_ok() && !raw.is_null() {
                let jd = IKsJackDescription::from_raw(raw);
                if jd.GetJackCount().unwrap_or(0) > 0 {
                    let mut d = KSJACK_DESCRIPTION::default();
                    if jd.GetJackDescription(0, &mut d).is_ok() {
                        return Some((d.ConnectionType.0, d.GeoLocation.0, d.GenLocation.0, d.IsConnected.as_bool()));
                    }
                }
            }
        }
    }
    None
}
