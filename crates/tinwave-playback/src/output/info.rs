//! What an opened stream looks like, for display and diagnostics.

use super::convert::SampleFormat;

/// Shared (mixed by the OS, any device rate) or exclusive (the device is ours, integer samples).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    /// Mixed by the operating system.
    Shared,
    /// Exclusive access to the endpoint (bit-perfect, `ExclusiveModeToggled`).
    Exclusive,
}

/// Facts about an open stream. Nothing here is a signal; `tinwave-context` owns those.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// `"wasapi"`, `"cpal"`, `"manual"`, or `"unknown"`.
    pub backend: &'static str,
    /// Endpoint id the stream was opened on (empty when unknown).
    pub device_id: String,
    /// Endpoint display name (empty when unknown).
    pub device_name: String,
    /// Shared or exclusive.
    pub mode: StreamMode,
    /// Sample format written to the device.
    pub format: SampleFormat,
    /// Frames per device period (callback granularity); 0 when unknown.
    pub period_frames: u32,
    /// Frames in the device buffer; 0 when unknown.
    pub buffer_frames: u32,
    /// Extra latency the OS reports beyond the buffer, in frames; 0 when unknown.
    pub stream_latency_frames: u32,
    /// The callback thread runs with MMCSS "Pro Audio" scheduling (Windows).
    pub realtime_priority: bool,
    /// Why the stream is not what was asked for: exclusive mode refused so shared was used, or
    /// the priority boost failed. Visible to the user; also logged at `warn`.
    pub fallback: Option<String>,
}

impl StreamInfo {
    /// Placeholder for backends that cannot describe their stream.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            backend: "unknown",
            device_id: String::new(),
            device_name: String::new(),
            mode: StreamMode::Shared,
            format: SampleFormat::F32,
            period_frames: 0,
            buffer_frames: 0,
            stream_latency_frames: 0,
            realtime_priority: false,
            fallback: None,
        }
    }
}
