//! The seam between the OS and the provider logic.
//!
//! [`RawEvent`] is what the OS callbacks copy and send; [`Backend`] is what the provider thread
//! asks of the OS. The Windows implementation is `windows::WindowsBackend`; tests use a fake.
//! Everything above this seam is plain Rust and is tested without any OS.

use tinwave_core::UtcMillis;

use super::classify::{JackDescription, Transport};

/// The event-context GUID Tinwave passes whenever it changes *endpoint* volume or mute itself
/// (`pEventContext`). The volume callback compares it with `guidEventContext` to tell Tinwave's
/// own changes from the listener's (ADR 0004 section 2). One value for the whole process.
///
/// Tinwave's in-app volume must use the session (`ISimpleAudioVolume`) or software gain, never
/// the endpoint, so this is only needed by code that must touch the endpoint.
pub const EVENT_CONTEXT_GUID: u128 = 0x4e5a_9c1d_7b3e_4f60_a1d2_0b8c_6f3e_9a47;

/// Data flow of an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Render,
    Capture,
    Other,
}

/// The role a default device was changed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Console,
    Multimedia,
    Communications,
    Other,
}

/// State of an endpoint (`DEVICE_STATE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceState {
    Active,
    Disabled,
    NotPresent,
    Unplugged,
    Other,
}

/// One OS notification, copied inside the callback. Callbacks make no COM call: everything the
/// provider thread needs is in the event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawEvent {
    /// `IAudioEndpointVolumeCallback::OnNotify`.
    Volume {
        /// When the callback ran.
        at: UtcMillis,
        /// Which binding registered the callback; stale ones are dropped.
        generation: u64,
        /// `guidEventContext` as an integer; `0` is `GUID_NULL`.
        context: u128,
        /// `fMasterVolume`, the 0..=1 slider scalar.
        scalar: f32,
        /// `bMuted`.
        muted: bool,
    },
    /// `IMMNotificationClient::OnDefaultDeviceChanged`. Fires once per role.
    DefaultChanged {
        at: UtcMillis,
        flow: Flow,
        role: Role,
        /// The new default endpoint; `None` when there is no device any more.
        id: Option<String>,
    },
    /// `IMMNotificationClient::OnDeviceStateChanged`.
    StateChanged {
        at: UtcMillis,
        id: String,
        state: DeviceState,
    },
    /// `IMMNotificationClient::OnDeviceRemoved`.
    Removed { at: UtcMillis, id: String },
}

/// What to bind the volume notification to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindTarget<'a> {
    /// The default render endpoint for the multimedia role.
    SystemDefault,
    /// A specific endpoint id.
    Device(&'a str),
}

/// What the OS reports about an endpoint: the classifier's inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeviceDescription {
    pub form_factor: Option<u32>,
    pub transport: Transport,
    pub jack: Option<JackDescription>,
}

/// An endpoint the volume notification is now bound to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BoundEndpoint {
    pub id: String,
    pub description: DeviceDescription,
    /// The slider scalar right now.
    pub volume_scalar: f32,
    pub muted: bool,
}

/// Why binding failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BindError {
    /// There is no render endpoint (e.g. a remote session without audio).
    #[error("no render endpoint available")]
    NoDevice,
    /// The endpoint exists but is not active (unplugged, disabled, not present).
    #[error("the endpoint is not active")]
    NotActive,
    /// The OS refused; the text is the OS error.
    #[error("the OS refused: {0}")]
    Os(String),
}

/// What the provider thread asks of the OS. Every call happens on the provider thread.
pub(crate) trait Backend {
    /// Registers the volume notification on `target` (the previous registration is already gone:
    /// [`unbind`](Self::unbind) is called first) and describes the endpoint.
    fn bind(&mut self, target: BindTarget<'_>, generation: u64)
        -> Result<BoundEndpoint, BindError>;

    /// Removes the volume notification. Errors are ignored: the device may already be gone.
    fn unbind(&mut self);

    /// The bound endpoint's master gain in dB, if it reports one.
    fn gain_db(&mut self) -> Option<f32>;
}
