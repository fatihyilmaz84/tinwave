//! The provider contract and the fields providers fill (docs/guides/context-engine.md).
//!
//! `docs/rules/architecture.md` places the `ContextProvider` trait in `tinwave-signals`. That
//! crate is outside the scope of the ticket that introduced the first providers (T-0014), so the
//! trait and the audio fields live here until a ticket moves them down; the shapes follow the
//! guide so the move is mechanical.

use std::time::Duration;

use tinwave_core::Tier;
use tinwave_signals::{OutputClass, OutputTransport, Volume};

/// Which provider a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProviderId {
    /// The output device (class, transport, form factor).
    OutputDevice,
    /// The system volume and mute state.
    SystemVolume,
}

/// What a provider can do on this machine. Providers report this instead of panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Works.
    Supported,
    /// Not implemented for this OS (or this OS version); the reason is for the UI.
    Unsupported(&'static str),
    /// Works once the user grants a permission.
    NeedsPermission,
}

/// The output device as the listener would describe it. Absent (`None` in [`ContextFields`])
/// when there is no render device or binding failed: never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputDeviceFields {
    /// `Headphones|Speakers|Bluetooth|UsbDac|Hdmi|Builtin|Virtual|Unknown`.
    pub class: OutputClass,
    /// How the endpoint is attached. Kept next to `class` so "headphone-ness" and "link" can be
    /// derived separately (ADR 0004, lead decision 1).
    pub transport: OutputTransport,
    /// The OS form-factor value; `None` if it could not be read.
    pub form_factor: Option<u8>,
}

/// The system (endpoint) volume of the bound device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemVolumeFields {
    /// The slider position, quantized to 1 % before it is stored.
    pub volume: Volume,
    /// Whether the endpoint is muted.
    pub muted: bool,
    /// The endpoint gain in dB, quantized to 0.25 dB; `None` if the device does not report it.
    pub endpoint_gain_db: Option<f32>,
    /// `true` for `Hdmi` and `Bluetooth` endpoints: those sinks may apply gain Windows cannot
    /// see, so `PerceivedLevel` must treat `endpoint_gain_db` as uncertain (ADR 0004).
    pub gain_uncertain: bool,
}

/// The values providers write into on [`ContextProvider::snapshot`]. A provider only touches
/// its own field.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ContextFields {
    /// Set by the output-device provider.
    pub output_device: Option<OutputDeviceFields>,
    /// Set by the system-volume provider.
    pub system_volume: Option<SystemVolumeFields>,
}

/// A source of context. See `docs/guides/context-engine.md`.
pub trait ContextProvider: Send + Sync {
    /// Which provider this is.
    fn id(&self) -> ProviderId;
    /// Privacy tier; a disabled tier means the provider is not constructed.
    fn tier(&self) -> Tier;
    /// How stale a cached value may be. Event-driven providers are never stale: `Duration::MAX`.
    fn max_age(&self) -> Duration;
    /// Copies the cached value into `out`. Returns in microseconds: no OS call, no lock, no IO.
    fn snapshot(&self, out: &mut ContextFields);
    /// What this provider can do here.
    fn capability(&self) -> Capability;
}
