//! Output-device classification: a pure function of what the OS tells us (ADR 0004 section 6).
//!
//! Inputs, all readable without elevation:
//! - the endpoint **form factor** (`PKEY_AudioEndpoint_FormFactor`, the `EndpointFormFactor` enum),
//! - the **transport**, parsed from the interface path of the adapter the endpoint's connector is
//!   wired to (`IDeviceTopology` -> `GetConnector(0)` -> `GetDeviceIdConnectedTo()`),
//! - whether the adapter-side jack is **internal** (`IKsJackDescription`).
//!
//! Nothing here calls the OS, so every row of the ADR table is a unit test.

use tinwave_signals::{OutputClass, OutputTransport};

/// How an endpoint is attached, from the enumerator segment of the adapter path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    /// `bthenum` / `bthhfenum` / `bthleenum`: A2DP, hands-free, LE audio.
    Bluetooth,
    /// `usb`.
    Usb,
    /// `hdaudio`: the onboard codec **and** GPU HDMI/DisplayPort audio.
    HdAudio,
    /// `root` / `swd`: software devices and virtual drivers.
    Virtual,
    /// Anything else, or no path at all.
    Unknown,
}

impl Transport {
    /// The catalog's `transport` value (`Bluetooth|Usb|Hda|Virtual|Other`).
    #[must_use]
    pub const fn signal(self) -> OutputTransport {
        match self {
            Self::Bluetooth => OutputTransport::Bluetooth,
            Self::Usb => OutputTransport::Usb,
            Self::HdAudio => OutputTransport::Hda,
            Self::Virtual => OutputTransport::Virtual,
            Self::Unknown => OutputTransport::Other,
        }
    }
}

/// `EndpointFormFactor` values (`mmdeviceapi.h`). Stored as-is in the signal's `form_factor` byte.
pub mod form_factor {
    /// `RemoteNetworkDevice`. Note: `0` is **not** "unknown".
    pub const REMOTE_NETWORK_DEVICE: u32 = 0;
    /// `Speakers`.
    pub const SPEAKERS: u32 = 1;
    /// `LineLevel`.
    pub const LINE_LEVEL: u32 = 2;
    /// `Headphones`.
    pub const HEADPHONES: u32 = 3;
    /// `Microphone` (capture only).
    pub const MICROPHONE: u32 = 4;
    /// `Headset`.
    pub const HEADSET: u32 = 5;
    /// `Handset`.
    pub const HANDSET: u32 = 6;
    /// `UnknownDigitalPassthrough`.
    pub const UNKNOWN_DIGITAL_PASSTHROUGH: u32 = 7;
    /// `SPDIF`.
    pub const SPDIF: u32 = 8;
    /// `DigitalAudioDisplayDevice`: HDMI / DisplayPort.
    pub const DIGITAL_AUDIO_DISPLAY_DEVICE: u32 = 9;
    /// `UnknownFormFactor`.
    pub const UNKNOWN: u32 = 10;
}

/// What `IKsJackDescription` says about the adapter-side connector (`KSJACK_DESCRIPTION`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JackDescription {
    /// `EPcxConnectionType`.
    pub connection_type: i32,
    /// `EPcxGeoLocation`.
    pub geo_location: i32,
    /// `EPcxGenLocation`.
    pub gen_location: i32,
}

impl JackDescription {
    /// `EPcxGenLocation::eGenLocInternal`.
    pub const GEN_LOCATION_INTERNAL: i32 = 1;
    /// `EPcxConnectionType::eConnTypeAtapiInternal`.
    pub const CONNECTION_ATAPI_INTERNAL: i32 = 3;
    /// `EPcxGeoLocation::eGeoLocInsideMobileLid`.
    pub const GEO_INSIDE_MOBILE_LID: i32 = 9;

    /// ADR 0004: internal if `GenLocation = Internal`, `ConnectionType = AtapiInternal` or
    /// `GeoLocation = InsideMobileLid`.
    #[must_use]
    pub const fn is_internal(self) -> bool {
        self.gen_location == Self::GEN_LOCATION_INTERNAL
            || self.connection_type == Self::CONNECTION_ATAPI_INTERNAL
            || self.geo_location == Self::GEO_INSIDE_MOBILE_LID
    }
}

/// Parses the transport out of a device-interface path such as
/// `{2}.\\?\usb#vid_0d8c&pid_0014...`, `{2}.\\?\bthenum#...` or `{2}.\\?\hdaudio#func_01...`.
///
/// The first segment is the enumerator. (`PKEY_Device_EnumeratorName` on the endpoint itself is
/// useless: it is always `MMDEVAPI`.) Also accepts instance-style ids (`HDAUDIO\FUNC_01...`).
#[must_use]
pub fn transport_from_connected_id(id: &str) -> Transport {
    let lower = id.to_ascii_lowercase();
    let rest = if let Some(i) = lower.find("?\\") {
        &lower[i + 2..]
    } else if let Some(i) = lower.find("}.") {
        &lower[i + 2..]
    } else {
        &lower[..]
    };
    let enumerator = rest.split(['#', '\\']).next().unwrap_or("");
    match enumerator {
        e if e.starts_with("bth") => Transport::Bluetooth,
        "usb" => Transport::Usb,
        "hdaudio" => Transport::HdAudio,
        "root" | "swd" => Transport::Virtual,
        _ => Transport::Unknown,
    }
}

/// `class = f(transport, form_factor, internal)`. First match wins (ADR 0004 rule order):
///
/// 1. transport Bluetooth -> `Bluetooth`
/// 2. transport USB -> `UsbDac`
/// 3. transport virtual -> `Virtual`
/// 4. form factor `DigitalAudioDisplayDevice` -> `Hdmi`
/// 5. form factor `SPDIF` / `UnknownDigitalPassthrough` -> `UsbDac` (an external digital sink:
///    the class means "external DAC", USB or digital out)
/// 6. form factor `Headphones` / `Headset` / `Handset` -> `Headphones`
/// 7. form factor `Speakers` / `LineLevel` -> `Builtin` if HDAUDIO and an internal jack, else `Speakers`
/// 8. anything else -> `Unknown`
///
/// The link wins over the form factor because a Bluetooth headset reports `Headphones` and a USB
/// DAC reports `Speakers`.
#[must_use]
pub fn classify(
    form_factor: Option<u32>,
    transport: Transport,
    internal_jack: bool,
) -> OutputClass {
    use self::form_factor as ff;
    match transport {
        Transport::Bluetooth => return OutputClass::Bluetooth,
        Transport::Usb => return OutputClass::UsbDac,
        Transport::Virtual => return OutputClass::Virtual,
        Transport::HdAudio | Transport::Unknown => {}
    }
    match form_factor {
        Some(ff::DIGITAL_AUDIO_DISPLAY_DEVICE) => OutputClass::Hdmi,
        Some(ff::SPDIF | ff::UNKNOWN_DIGITAL_PASSTHROUGH) => OutputClass::UsbDac,
        Some(ff::HEADPHONES | ff::HEADSET | ff::HANDSET) => OutputClass::Headphones,
        Some(ff::SPEAKERS | ff::LINE_LEVEL) => {
            if transport == Transport::HdAudio && internal_jack {
                OutputClass::Builtin
            } else {
                OutputClass::Speakers
            }
        }
        _ => OutputClass::Unknown,
    }
}

/// The signal's `form_factor` byte: the raw OS value, `UnknownFormFactor` (10) when unreadable.
#[must_use]
pub fn form_factor_byte(form_factor: Option<u32>) -> u8 {
    form_factor
        .and_then(|v| u8::try_from(v).ok())
        .unwrap_or(u8::try_from(form_factor::UNKNOWN).unwrap_or(10))
}

#[cfg(test)]
mod tests {
    use super::form_factor as ff;
    use super::*;

    /// One fixture: a device as the OS reports it.
    struct Fixture {
        name: &'static str,
        form_factor: Option<u32>,
        adapter_path: &'static str,
        jack: Option<JackDescription>,
        transport: Transport,
        class: OutputClass,
    }

    const fn jack(
        connection_type: i32,
        geo_location: i32,
        gen_location: i32,
    ) -> Option<JackDescription> {
        Some(JackDescription {
            connection_type,
            geo_location,
            gen_location,
        })
    }

    /// `EPcxConnectionType` / `EPcxGeoLocation` / `EPcxGenLocation` values used below.
    const CONN_3_5MM: i32 = 1;
    const CONN_OTHER_DIGITAL: i32 = 6;
    const GEO_REAR: i32 = 1;
    const GEO_FRONT: i32 = 2;
    const GEO_HDMI: i32 = 11;
    const GEO_NOT_APPLICABLE: i32 = 14;
    const GEN_PRIMARY_BOX: i32 = 0;
    const GEN_OTHER: i32 = 3;

    fn fixtures() -> Vec<Fixture> {
        vec![
            // --- ADR 0004 table: rows measured on the dev PC ---
            Fixture {
                name: "Realtek line-out (desktop, active)",
                form_factor: Some(ff::SPEAKERS),
                adapter_path: r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0897&subsys_1458a182#5&1&0&0001#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\eqdsp",
                jack: jack(CONN_3_5MM, GEO_REAR, GEN_PRIMARY_BOX),
                transport: Transport::HdAudio,
                class: OutputClass::Speakers,
            },
            Fixture {
                name: "Realtek front headphone jack (not present)",
                form_factor: Some(ff::HEADPHONES),
                adapter_path: r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0897&subsys_1458a182#5&1&0&0001#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\eqdsp",
                jack: jack(CONN_3_5MM, GEO_FRONT, GEN_PRIMARY_BOX),
                transport: Transport::HdAudio,
                class: OutputClass::Headphones,
            },
            Fixture {
                name: "Monitor over DisplayPort/HDMI, NVIDIA HDA (active)",
                form_factor: Some(ff::DIGITAL_AUDIO_DISPLAY_DEVICE),
                adapter_path: r"{2}.\\?\hdaudio#func_01&ven_10de&dev_009f&subsys_00000000#5&2&0&0003#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\eqdsp",
                jack: jack(CONN_OTHER_DIGITAL, GEO_HDMI, GEN_OTHER),
                transport: Transport::HdAudio,
                class: OutputClass::Hdmi,
            },
            Fixture {
                name: "AMD GPU HDMI output (not present)",
                form_factor: Some(ff::DIGITAL_AUDIO_DISPLAY_DEVICE),
                adapter_path: r"{2}.\\?\hdaudio#func_01&ven_1002&dev_aab8&subsys_00aab800#5&3&0&0001#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\eqdsp",
                jack: None,
                transport: Transport::HdAudio,
                class: OutputClass::Hdmi,
            },
            Fixture {
                name: "Steam Streaming Speakers (virtual driver, active)",
                form_factor: Some(ff::SPEAKERS),
                adapter_path: r"{2}.\\?\root#steamstreamingspeakers#0000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\global",
                jack: None,
                transport: Transport::Virtual,
                class: OutputClass::Virtual,
            },
            Fixture {
                name: "Sony WH-1000XM4 A2DP (paired, disconnected)",
                form_factor: Some(ff::HEADPHONES),
                adapter_path: r"{2}.\\?\bthenum#{0000110b-0000-1000-8000-00805f9b34fb}_vid&0001012d_pid&0d58#7&1a2b3c4d&0&000000000000_c00000000#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\global",
                jack: jack(CONN_OTHER_DIGITAL, GEO_NOT_APPLICABLE, GEN_OTHER),
                transport: Transport::Bluetooth,
                class: OutputClass::Bluetooth,
            },
            Fixture {
                name: "Sony WH-1000XM4 hands-free (headset, disconnected)",
                form_factor: Some(ff::HEADSET),
                adapter_path: r"{2}.\\?\bthhfenum#bthhfpaudio#8&2a1b3c4d&0&97#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\global",
                jack: None,
                transport: Transport::Bluetooth,
                class: OutputClass::Bluetooth,
            },
            Fixture {
                name: "USB webcam microphone (capture; the USB path still parses)",
                form_factor: Some(ff::MICROPHONE),
                adapter_path: r"{2}.\\?\usb#vid_046d&pid_085c&mi_02#7&1b2c3d4e&0&0002#{6994ad04-93ef-11d0-a3cc-00a0c9223196}\global",
                jack: None,
                transport: Transport::Usb,
                class: OutputClass::UsbDac,
            },
            // --- ADR 0004 table: rows derived from the documented enums (unverified, runbook) ---
            Fixture {
                name: "USB DAC (speakers form factor)",
                form_factor: Some(ff::SPEAKERS),
                adapter_path: r"{2}.\\?\usb#vid_0d8c&pid_0014&mi_00#6&1&0&0000#{6994ad04}\global",
                jack: None,
                transport: Transport::Usb,
                class: OutputClass::UsbDac,
            },
            Fixture {
                name: "USB headset (headphones form factor)",
                form_factor: Some(ff::HEADPHONES),
                adapter_path: r"{2}.\\?\usb#vid_0d8c&pid_0014&mi_00#6&1&0&0000#{6994ad04}\global",
                jack: None,
                transport: Transport::Usb,
                class: OutputClass::UsbDac,
            },
            Fixture {
                name: "Laptop internal speakers",
                form_factor: Some(ff::SPEAKERS),
                adapter_path: r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0256#4&1&0&0001#{6994ad04}\eqdsp",
                jack: jack(3, 9, 1),
                transport: Transport::HdAudio,
                class: OutputClass::Builtin,
            },
            Fixture {
                name: "Laptop 3.5 mm headphones",
                form_factor: Some(ff::HEADPHONES),
                adapter_path: r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0256#4&1&0&0001#{6994ad04}\eqdsp",
                jack: jack(CONN_3_5MM, GEO_FRONT, 3),
                transport: Transport::HdAudio,
                class: OutputClass::Headphones,
            },
            Fixture {
                name: "Optical / S-PDIF out",
                form_factor: Some(ff::SPDIF),
                adapter_path: r"{2}.\\?\hdaudio#func_01&ven_10ec&dev_0897#5&1&0&0001#{6994ad04}\eqdsp",
                jack: jack(5, 1, 0),
                transport: Transport::HdAudio,
                class: OutputClass::UsbDac,
            },
            Fixture {
                name: "Bluetooth speaker (A2DP, speakers form factor)",
                form_factor: Some(ff::SPEAKERS),
                adapter_path: r"{2}.\\?\bthenum#{0000110b-0000-1000-8000-00805f9b34fb}_vid&0001004c#7&1&0&000000000000_c00000000#{6994ad04}\global",
                jack: None,
                transport: Transport::Bluetooth,
                class: OutputClass::Bluetooth,
            },
            Fixture {
                name: "Remote Desktop / cast (terminal-services audio)",
                form_factor: Some(ff::REMOTE_NETWORK_DEVICE),
                adapter_path: r"{2}.\\?\ts#tsaudio#0001#{6994ad04}\global",
                jack: None,
                transport: Transport::Unknown,
                class: OutputClass::Unknown,
            },
        ]
    }

    #[test]
    fn every_adr_table_row_classifies_as_documented() {
        for f in fixtures() {
            let transport = transport_from_connected_id(f.adapter_path);
            assert_eq!(transport, f.transport, "transport of {}", f.name);
            let internal = f.jack.is_some_and(JackDescription::is_internal);
            assert_eq!(
                classify(f.form_factor, transport, internal),
                f.class,
                "class of {}",
                f.name
            );
        }
    }

    #[test]
    fn transport_parsing_covers_every_enumerator_in_the_adr() {
        let cases = [
            (
                r"{2}.\\?\bthenum#{0000110b}#7&1&0#{6994ad04}\global",
                Transport::Bluetooth,
            ),
            (
                r"{2}.\\?\bthhfenum#bthhfpaudio#8&1&0&97#{6994ad04}\global",
                Transport::Bluetooth,
            ),
            (
                r"{2}.\\?\bthleenum#dev#1#{6994ad04}\global",
                Transport::Bluetooth,
            ),
            (
                r"{2}.\\?\usb#vid_0d8c&pid_0014&mi_00#6&1&0&0000#{6994ad04}\global",
                Transport::Usb,
            ),
            (
                r"{2}.\\?\hdaudio#func_01&ven_10de&dev_009f#5&1&0&0001#{6994ad04}\eqdsp",
                Transport::HdAudio,
            ),
            (
                r"{2}.\\?\swd#mmdevapi#{0.0.0.00000000}#{6994ad04}\global",
                Transport::Virtual,
            ),
            (
                r"{2}.\\?\root#steamstreamingspeakers#0000#{6994ad04}\global",
                Transport::Virtual,
            ),
            (r"HDAUDIO\FUNC_01&VEN_10EC", Transport::HdAudio),
            (r"USB\VID_0D8C&PID_0014", Transport::Usb),
            (r"{2}.\\?\somethingnew#x#y", Transport::Unknown),
            ("", Transport::Unknown),
        ];
        for (path, expected) in cases {
            assert_eq!(transport_from_connected_id(path), expected, "{path}");
        }
    }

    #[test]
    fn the_link_wins_over_the_form_factor() {
        for f in [
            ff::HEADPHONES,
            ff::HEADSET,
            ff::SPEAKERS,
            ff::DIGITAL_AUDIO_DISPLAY_DEVICE,
        ] {
            assert_eq!(
                classify(Some(f), Transport::Bluetooth, false),
                OutputClass::Bluetooth
            );
            assert_eq!(
                classify(Some(f), Transport::Usb, false),
                OutputClass::UsbDac
            );
            assert_eq!(
                classify(Some(f), Transport::Virtual, true),
                OutputClass::Virtual
            );
        }
        // Bluetooth is checked before USB: both can never be true, but the order is the contract.
        assert_eq!(
            classify(None, Transport::Bluetooth, true),
            OutputClass::Bluetooth
        );
    }

    #[test]
    fn form_factor_rules_in_adr_order() {
        let hda = Transport::HdAudio;
        assert_eq!(
            classify(Some(ff::DIGITAL_AUDIO_DISPLAY_DEVICE), hda, true),
            OutputClass::Hdmi
        );
        assert_eq!(classify(Some(ff::SPDIF), hda, true), OutputClass::UsbDac);
        assert_eq!(
            classify(Some(ff::UNKNOWN_DIGITAL_PASSTHROUGH), hda, false),
            OutputClass::UsbDac
        );
        for f in [ff::HEADPHONES, ff::HEADSET, ff::HANDSET] {
            // Headphones stay headphones, even on an internal jack.
            assert_eq!(classify(Some(f), hda, true), OutputClass::Headphones);
        }
        assert_eq!(
            classify(Some(ff::SPEAKERS), hda, true),
            OutputClass::Builtin
        );
        assert_eq!(
            classify(Some(ff::LINE_LEVEL), hda, true),
            OutputClass::Builtin
        );
        // A desktop line-out (external jack) is not Builtin, and Builtin needs HDAUDIO.
        assert_eq!(
            classify(Some(ff::SPEAKERS), hda, false),
            OutputClass::Speakers
        );
        assert_eq!(
            classify(Some(ff::SPEAKERS), Transport::Unknown, true),
            OutputClass::Speakers
        );
    }

    #[test]
    fn anything_else_is_unknown() {
        let hda = Transport::HdAudio;
        assert_eq!(classify(None, hda, true), OutputClass::Unknown);
        assert_eq!(classify(Some(ff::UNKNOWN), hda, true), OutputClass::Unknown);
        assert_eq!(
            classify(Some(ff::MICROPHONE), hda, false),
            OutputClass::Unknown
        );
        assert_eq!(
            classify(Some(ff::REMOTE_NETWORK_DEVICE), Transport::Unknown, false),
            OutputClass::Unknown
        );
        assert_eq!(classify(Some(99), hda, false), OutputClass::Unknown);
        assert_eq!(
            classify(None, Transport::Unknown, false),
            OutputClass::Unknown
        );
    }

    #[test]
    fn internal_jack_rule() {
        assert!(jack(0, 0, JackDescription::GEN_LOCATION_INTERNAL)
            .is_some_and(JackDescription::is_internal));
        assert!(jack(JackDescription::CONNECTION_ATAPI_INTERNAL, 0, 0)
            .is_some_and(JackDescription::is_internal));
        assert!(jack(0, JackDescription::GEO_INSIDE_MOBILE_LID, 0)
            .is_some_and(JackDescription::is_internal));
        assert!(
            !jack(CONN_3_5MM, GEO_REAR, GEN_PRIMARY_BOX).is_some_and(JackDescription::is_internal)
        );
        assert!(!jack(CONN_OTHER_DIGITAL, GEO_NOT_APPLICABLE, GEN_OTHER)
            .is_some_and(JackDescription::is_internal));
    }

    #[test]
    fn form_factor_byte_keeps_the_raw_value_and_maps_unreadable_to_10() {
        assert_eq!(
            form_factor_byte(Some(0)),
            0,
            "RemoteNetworkDevice is not unknown"
        );
        assert_eq!(form_factor_byte(Some(9)), 9);
        assert_eq!(form_factor_byte(None), 10);
        assert_eq!(form_factor_byte(Some(70_000)), 10);
    }

    #[test]
    fn transport_maps_to_the_signal_enum() {
        assert_eq!(Transport::Bluetooth.signal(), OutputTransport::Bluetooth);
        assert_eq!(Transport::Usb.signal(), OutputTransport::Usb);
        assert_eq!(Transport::HdAudio.signal(), OutputTransport::Hda);
        assert_eq!(Transport::Virtual.signal(), OutputTransport::Virtual);
        assert_eq!(Transport::Unknown.signal(), OutputTransport::Other);
    }
}
