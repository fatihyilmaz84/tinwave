//! Pure device classification (no OS calls) so it is unit-testable.
//!
//! Inputs come from two places:
//! * `PKEY_AudioEndpoint_FormFactor` on the MMDevice property store (EndpointFormFactor enum).
//! * The transport, parsed from the device id of the adapter the endpoint's connector is wired to
//!   (`IDeviceTopology` -> `IConnector::GetDeviceIdConnectedTo`), e.g. `{2}.\\?\bthenum#...`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Bluetooth,
    Usb,
    HdAudio, // onboard codec or GPU HDMI/DP audio (both enumerate as HDAUDIO)
    Virtual, // software devices (SWD / ROOT), RDP, virtual cables
    Unknown,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Bluetooth => "Bluetooth",
            Transport::Usb => "Usb",
            Transport::HdAudio => "HdAudio",
            Transport::Virtual => "Virtual",
            Transport::Unknown => "Unknown",
        }
    }
}

/// The `class` enum from the signal catalog (`OutputDeviceChanged`), plus `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Headphones,
    Speakers,
    Bluetooth,
    UsbDac,
    Hdmi,
    Virtual,
    Builtin,
    Unknown,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Headphones => "Headphones",
            Class::Speakers => "Speakers",
            Class::Bluetooth => "Bluetooth",
            Class::UsbDac => "UsbDac",
            Class::Hdmi => "Hdmi",
            Class::Virtual => "Virtual",
            Class::Builtin => "Builtin",
            Class::Unknown => "Unknown",
        }
    }
}

// EndpointFormFactor values (mmdeviceapi.h)
pub const FF_REMOTE_NETWORK: u32 = 0;
pub const FF_SPEAKERS: u32 = 1;
pub const FF_LINE_LEVEL: u32 = 2;
pub const FF_HEADPHONES: u32 = 3;
pub const FF_MICROPHONE: u32 = 4;
pub const FF_HEADSET: u32 = 5;
pub const FF_HANDSET: u32 = 6;
pub const FF_UNKNOWN_DIGITAL: u32 = 7;
pub const FF_SPDIF: u32 = 8;
pub const FF_DIGITAL_DISPLAY: u32 = 9; // HDMI / DisplayPort
pub const FF_UNKNOWN: u32 = 10;

pub fn form_factor_name(ff: u32) -> &'static str {
    match ff {
        FF_REMOTE_NETWORK => "RemoteNetworkDevice",
        FF_SPEAKERS => "Speakers",
        FF_LINE_LEVEL => "LineLevel",
        FF_HEADPHONES => "Headphones",
        FF_MICROPHONE => "Microphone",
        FF_HEADSET => "Headset",
        FF_HANDSET => "Handset",
        FF_UNKNOWN_DIGITAL => "UnknownDigitalPassthrough",
        FF_SPDIF => "SPDIF",
        FF_DIGITAL_DISPLAY => "DigitalAudioDisplayDevice",
        FF_UNKNOWN => "UnknownFormFactor",
        _ => "?",
    }
}

/// Parse the transport out of a device-interface id such as
/// `{2}.\\?\usb#vid_0d8c&pid_0014...`, `{2}.\\?\bthenum#...` or `{2}.\\?\hdaudio#func_01...`.
/// Also accepts bare `HDAUDIO\FUNC_01...` / `USB\VID_...` instance-style ids.
pub fn transport_from_connected_id(id: &str) -> Transport {
    let l = id.to_ascii_lowercase();
    let rest = if let Some(i) = l.find("?\\") {
        &l[i + 2..]
    } else if let Some(i) = l.find("}.") {
        &l[i + 2..]
    } else {
        &l[..]
    };
    let enumerator = rest.split(['#', '\\']).next().unwrap_or("");
    match enumerator {
        e if e.starts_with("bth") => Transport::Bluetooth, // bthenum, bthhfenum, bthleenum
        "usb" => Transport::Usb,
        "hdaudio" | "pci" | "intelaudio" | "acpi" => Transport::HdAudio,
        "swd" | "root" | "mmdevapi" | "ts" | "tsaudio" => Transport::Virtual,
        _ => Transport::Unknown,
    }
}

/// Priority: the *link* wins over form factor, because a Bluetooth headset reports form factor
/// Headphones/Headset and a USB DAC reports Speakers/Headphones.
pub fn classify(form_factor: Option<u32>, transport: Transport, internal: bool) -> Class {
    match transport {
        Transport::Bluetooth => return Class::Bluetooth,
        Transport::Usb => return Class::UsbDac,
        Transport::Virtual => return Class::Virtual,
        _ => {}
    }
    match form_factor {
        Some(FF_DIGITAL_DISPLAY) => Class::Hdmi,
        // external digital sink (judgement call: closest catalog class is an external DAC)
        Some(FF_SPDIF) | Some(FF_UNKNOWN_DIGITAL) => Class::UsbDac,
        Some(FF_HEADPHONES) | Some(FF_HEADSET) | Some(FF_HANDSET) => Class::Headphones,
        Some(FF_SPEAKERS) | Some(FF_LINE_LEVEL) => match transport {
            Transport::HdAudio if internal => Class::Builtin,
            _ => Class::Speakers,
        },
        Some(FF_REMOTE_NETWORK) => Class::Unknown,
        Some(FF_UNKNOWN) | None => match transport {
            Transport::HdAudio if internal => Class::Builtin,
            _ => Class::Unknown,
        },
        Some(_) => Class::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_parsing() {
        assert_eq!(
            transport_from_connected_id(
                r"{2}.\\?\bthenum#{0000110b-0000-1000-8000-00805f9b34fb}_dev_vid&01004c#7&1&0&abc#{6994ad04}\global"
            ),
            Transport::Bluetooth
        );
        assert_eq!(
            transport_from_connected_id(r"{2}.\\?\bthhfenum#bthhfaudio#8&1&0&97#{6994ad04}\global"),
            Transport::Bluetooth
        );
        assert_eq!(
            transport_from_connected_id(r"{2}.\\?\usb#vid_0d8c&pid_0014&mi_00#6&1&0&0000#{6994ad04}\global"),
            Transport::Usb
        );
        assert_eq!(
            transport_from_connected_id(r"{2}.\\?\hdaudio#func_01&ven_10de&dev_009f#5&1&0&0001#{6994ad04}\eqdsp"),
            Transport::HdAudio
        );
        assert_eq!(
            transport_from_connected_id(r"{2}.\\?\swd#mmdevapi#{0.0.0.00000000}#{6994ad04}\global"),
            Transport::Virtual
        );
        assert_eq!(transport_from_connected_id(r"HDAUDIO\FUNC_01&VEN_10EC"), Transport::HdAudio);
        assert_eq!(transport_from_connected_id(""), Transport::Unknown);
    }

    #[test]
    fn class_priority() {
        assert_eq!(classify(Some(FF_HEADPHONES), Transport::Bluetooth, false), Class::Bluetooth);
        assert_eq!(classify(Some(FF_SPEAKERS), Transport::Usb, false), Class::UsbDac);
        assert_eq!(classify(Some(FF_DIGITAL_DISPLAY), Transport::HdAudio, false), Class::Hdmi);
        assert_eq!(classify(Some(FF_HEADPHONES), Transport::HdAudio, false), Class::Headphones);
        assert_eq!(classify(Some(FF_SPEAKERS), Transport::HdAudio, true), Class::Builtin);
        assert_eq!(classify(Some(FF_SPEAKERS), Transport::Virtual, false), Class::Virtual);
        // desktop line-out jack (external speakers) must not be called Builtin
        assert_eq!(classify(Some(FF_SPEAKERS), Transport::HdAudio, false), Class::Speakers);
        assert_eq!(classify(None, Transport::Unknown, false), Class::Unknown);
    }
}
