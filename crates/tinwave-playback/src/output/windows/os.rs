//! The only `unsafe` in the crate: two Windows calls that the `wasapi` crate does not wrap.
//!
//! - MMCSS: `AvSetMmThreadCharacteristicsW("Pro Audio")` on the callback thread, reverted on drop.
//! - `IAudioClient::GetStreamLatency`, which the render loop adds to its position math.
//!
//! Nothing here touches endpoint volume or mute (ADR 0004 decision 5); if that is ever needed,
//! the call must pass `tinwave_context::EVENT_CONTEXT_GUID`.
#![allow(unsafe_code)]

use ::windows::core::w;
use ::windows::Win32::Foundation::HANDLE;
use ::windows::Win32::Media::Audio::IAudioClient;
use ::windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
};

/// The calling thread is registered with MMCSS as a "Pro Audio" task; dropping reverts it.
/// Must be dropped on the thread that created it.
#[derive(Debug)]
pub(super) struct MmcssGuard(HANDLE);

impl MmcssGuard {
    /// Register the calling thread. The error text is for the stream's `fallback` note: the
    /// stream still works without the boost, but the user is told.
    pub(super) fn enter() -> Result<Self, String> {
        let mut task_index = 0u32;
        // SAFETY: `w!` yields a valid NUL-terminated UTF-16 string that outlives the call, and
        // `task_index` is a live, writable `u32` for the duration of the call.
        let result = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) };
        result
            .map(Self)
            .map_err(|e| format!("MMCSS Pro Audio refused: {e}"))
    }
}

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful `AvSetMmThreadCharacteristicsW` on this
        // thread and is reverted exactly once, here.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}

/// The latency the audio engine adds beyond the device buffer, in 100 ns units (0 if unknown).
pub(super) fn stream_latency_hns(client: &IAudioClient) -> i64 {
    // SAFETY: `client` is a live, initialised `IAudioClient`; the call only reads a property.
    unsafe { client.GetStreamLatency() }.unwrap_or(0)
}
