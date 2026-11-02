//! Generic output on `cpal` (Apache-2.0): macOS, Linux, and the Windows fallback.
//!
//! Shared mode only, `f32` stereo at the requested rate; the OS converts to the device. cpal owns
//! the audio thread and the format negotiation; this file adds the engine's contract on top:
//! the stream lives on a thread of its own (so `OutputStream` is `Send` on every platform), and
//! the [`Renderer`] is handed back to the thread that finishes with the stream instead of being
//! dropped inside cpal's callback machinery.
//!
//! Not available through cpal: exclusive mode, device-change notifications other than a stream
//! error (reported as `is_lost`), system-volume hooks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use ::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};

use super::convert::SampleFormat;
use super::info::{StreamInfo, StreamMode};
use super::{AudioOutput, OpenRequest, OutputStream};
use crate::types::CHANNELS;
use crate::{PlaybackError, Renderer};

/// How long the stream thread waits for the callback to hand the renderer back on stop.
const HANDBACK_WAIT: Duration = Duration::from_millis(250);
/// How long `open` waits for the stream thread to build and start the stream.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Observes a [`CpalOutput`] from outside the engine.
#[derive(Debug, Clone, Default)]
pub struct CpalControl {
    last: Arc<Mutex<Option<StreamInfo>>>,
}

impl CpalControl {
    /// How the most recent stream was opened.
    #[must_use]
    pub fn last_info(&self) -> Option<StreamInfo> {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The cpal-based [`AudioOutput`].
#[derive(Debug)]
pub struct CpalOutput {
    control: CpalControl,
}

impl CpalOutput {
    /// A new output and the handle to observe it.
    #[must_use]
    pub fn new() -> (Self, CpalControl) {
        let control = CpalControl::default();
        (
            Self {
                control: control.clone(),
            },
            control,
        )
    }
}

/// The data callback's state. Everything in it is touched without locks or allocation.
pub(crate) struct Callback {
    renderer: Option<Renderer>,
    stop: Arc<AtomicBool>,
    back: Producer<Renderer>,
}

impl Callback {
    /// One callback: render, or after `stop` hand the renderer back once and play silence.
    pub(crate) fn fill(&mut self, data: &mut [f32], latency_frames: u64) {
        if self.stop.load(Ordering::Acquire) {
            if let Some(renderer) = self.renderer.take() {
                // Capacity 1 and sent at most once: this cannot be full.
                let _ = self.back.push(renderer);
            }
            data.fill(0.0);
            return;
        }
        match self.renderer.as_mut() {
            Some(renderer) => {
                renderer.render(data, latency_frames);
            }
            None => data.fill(0.0),
        }
    }
}

struct Spec {
    device: Option<String>,
    rate: u32,
}

struct Stream {
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    info: StreamInfo,
    thread: Option<JoinHandle<Option<Renderer>>>,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalStream")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl OutputStream for Stream {
    fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    fn info(&self) -> StreamInfo {
        self.info.clone()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            // The renderer comes back here and is dropped by the thread that is dropping the
            // stream, never inside the audio callback. When this returns the stream is gone.
            let _ = thread.join();
        }
    }
}

impl AudioOutput for CpalOutput {
    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError> {
        let spec = Spec {
            device: request.device.map(str::to_owned),
            rate: request.sample_rate,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));
        let (opened_tx, opened_rx) = sync_channel::<Result<StreamInfo, String>>(1);
        let (back, returned) = RingBuffer::<Renderer>::new(1);
        let thread = {
            let (stop, lost) = (Arc::clone(&stop), Arc::clone(&lost));
            std::thread::Builder::new()
                .name("tinwave-cpal".into())
                .spawn(move || stream_thread(spec, renderer, stop, lost, back, returned, opened_tx))
                .map_err(|e| PlaybackError::Output(format!("cannot start cpal thread: {e}")))?
        };
        let outcome = opened_rx.recv_timeout(OPEN_TIMEOUT);
        match outcome {
            Ok(Ok(info)) => {
                *self
                    .control
                    .last
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(info.clone());
                Ok(Box::new(Stream {
                    stop,
                    lost,
                    info,
                    thread: Some(thread),
                }))
            }
            Ok(Err(message)) => {
                let _ = thread.join();
                Err(PlaybackError::Output(message))
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                thread.thread().unpark();
                Err(PlaybackError::Output(
                    "cpal: opening the stream timed out".into(),
                ))
            }
        }
    }
}

/// Builds the stream, reports, then keeps it alive until `stop`. Returns the renderer.
fn stream_thread(
    spec: Spec,
    renderer: Renderer,
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    back: Producer<Renderer>,
    mut returned: Consumer<Renderer>,
    opened: std::sync::mpsc::SyncSender<Result<StreamInfo, String>>,
) -> Option<Renderer> {
    let host = ::cpal::default_host();
    let device = match &spec.device {
        None => host.default_output_device(),
        Some(wanted) => host.output_devices().ok().and_then(|mut devices| {
            devices.find(|d| d.id().is_ok_and(|id| id.to_string() == *wanted))
        }),
    };
    let Some(device) = device else {
        let what = spec.device.as_deref().unwrap_or("the default output");
        let _ = opened.send(Err(format!("cpal: device not found: {what}")));
        return Some(renderer);
    };
    let device_id = device.id().map(|id| id.to_string()).unwrap_or_default();
    let device_name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_default();
    let config = ::cpal::StreamConfig {
        channels: CHANNELS as u16,
        sample_rate: spec.rate,
        buffer_size: ::cpal::BufferSize::Default,
    };
    let rate = spec.rate;
    let mut callback = Callback {
        renderer: Some(renderer),
        stop: Arc::clone(&stop),
        back,
    };
    let lost_in_error = Arc::clone(&lost);
    let built = device.build_output_stream(
        config,
        move |data: &mut [f32], info: &::cpal::OutputCallbackInfo| {
            let ts = info.timestamp();
            let ahead = ts
                .playback
                .checked_duration_since(ts.callback)
                .unwrap_or_default();
            // Latency to the end of this block: time to the first frame plus the block itself.
            let frames = (data.len() / CHANNELS) as u64;
            let latency = (ahead.as_secs_f64() * f64::from(rate)) as u64 + frames;
            callback.fill(data, latency);
        },
        move |error| {
            use ::cpal::ErrorKind::{DeviceChanged, DeviceNotAvailable, StreamInvalidated};
            tracing::warn!(%error, "cpal stream error");
            if matches!(
                error.kind(),
                DeviceNotAvailable | StreamInvalidated | DeviceChanged
            ) {
                lost_in_error.store(true, Ordering::Release);
            }
        },
        None,
    );
    let stream = match built.and_then(|s| s.play().map(|()| s)) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = opened.send(Err(format!("cpal: cannot open {device_name}: {error}")));
            // The callback (and the renderer inside it) went down with the failed build.
            return None;
        }
    };
    let info = StreamInfo {
        backend: "cpal",
        device_id,
        device_name,
        mode: StreamMode::Shared,
        format: SampleFormat::F32,
        period_frames: 0,
        buffer_frames: 0,
        stream_latency_frames: 0,
        realtime_priority: false,
        fallback: None,
    };
    let _ = opened.send(Ok(info));
    while !stop.load(Ordering::Acquire) {
        std::thread::park_timeout(Duration::from_millis(50));
    }
    // The callback hands the renderer back on its next run; wait a moment for it.
    let deadline = HANDBACK_WAIT.as_millis() / 5;
    let mut renderer = None;
    for _ in 0..deadline {
        if let Ok(r) = returned.pop() {
            renderer = Some(r);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(stream);
    renderer.or_else(|| returned.pop().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::Shared;

    #[test]
    fn the_callback_hands_the_renderer_back_exactly_once_after_stop() {
        let sh = Shared::new(1.0);
        let (mut prod, cons) = RingBuffer::<f32>::new(960);
        let _ = prod.push_partial_slice(&[0.5; 480]);
        let renderer = Renderer::new(cons, Arc::clone(&sh));
        let stop = Arc::new(AtomicBool::new(false));
        let (back, mut returned) = RingBuffer::<Renderer>::new(1);
        let mut cb = Callback {
            renderer: Some(renderer),
            stop: Arc::clone(&stop),
            back,
        };

        let mut data = [1.0f32; 480];
        cb.fill(&mut data, 0);
        assert!(data.iter().all(|&s| s == 0.5), "plays what the ring holds");
        assert!(returned.pop().is_err(), "not handed back while running");

        stop.store(true, Ordering::Release);
        cb.fill(&mut data, 0);
        assert!(data.iter().all(|&s| s == 0.0), "silence after stop");
        assert!(returned.pop().is_ok(), "renderer handed back");
        cb.fill(&mut data, 0);
        assert!(returned.pop().is_err(), "only once");
    }
}
