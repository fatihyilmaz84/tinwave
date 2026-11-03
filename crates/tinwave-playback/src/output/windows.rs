//! Direct WASAPI output (Windows): shared by default, exclusive on request, event driven, MMCSS.
//!
//! Layout: this file is safe Rust on top of the `wasapi` crate. The only `unsafe` is in
//! [`os`] (MMCSS and `GetStreamLatency`). The device-independent callback logic is
//! [`Pump`], the format choice is [`plan_exclusive`], and "should the stream follow this device
//! change?" is [`DeviceFollower`]; all three are tested without a device.
//!
//! # Threads
//! `open` starts one thread, `tinwave-wasapi`. It initialises COM (MTA), joins MMCSS "Pro Audio",
//! builds the stream, then loops on the device's event. The audio engine's
//! `IMMNotificationClient` callbacks arrive on Windows' own threads and only set `lost`.
//!
//! # Exclusive mode
//! `WasapiControl::set_exclusive(true)` followed by `Engine::switch_device` re-opens the stream
//! exclusive. The driver is asked for 24-in-32, then 32, then 16 bit integers; if none is
//! accepted, or the endpoint refuses (in use by another exclusive client, exclusive disabled in
//! Windows), the stream falls back to shared and `StreamInfo::fallback` says why.
//!
//! While exclusive mode is requested the engine opens the stream at each file's own sample rate
//! (`AudioOutput::wants_exclusive`, T-0057). A refused rate comes back as a shared stream at
//! that rate (the OS converts); the engine recognises it by its mode, closes it and opens again
//! at its configured rate, where its own resampler converts.
//!
//! # What this module does not do
//! It never writes endpoint volume or mute (in-app volume is the renderer's software gain,
//! ADR 0004 decision 5), and it emits no signals: `is_lost()` is the only output of the device
//! notifications, and the host decides to `switch_device`.

mod os;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use ::wasapi::{
    deinitialize, initialize_mta, AudioClient, AudioRenderClient, DeviceEnumerator,
    DeviceEventCallbacks, DeviceEventRegistration, DeviceState, Direction, Handle, Role,
    SampleType, StreamMode as WasapiMode, WaveFormat,
};

use super::convert::SampleFormat;
use super::follow::{DeviceEvent, DeviceFollower, DeviceStateChange, FollowTarget};
use super::info::{StreamInfo, StreamMode};
use super::plan::{plan_exclusive, resolve_mode};
use super::pump::{DeviceLost, RenderDevice};
use super::stream_thread::{run_render_thread, DeviceSetup, OpenState};
use super::{AudioOutput, OpenRequest, OutputStream};
use crate::types::CHANNELS;
use crate::{PlaybackError, Renderer};

/// How long the render thread waits for the device's event before checking the device is alive.
const WAIT_TIMEOUT_MS: u32 = 100;
/// How long `open` waits for the render thread to build and prime the stream.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
struct ControlInner {
    exclusive: AtomicBool,
    last: Mutex<Option<StreamInfo>>,
}

/// Steers and observes a [`WasapiOutput`] from outside the engine (the output itself is owned by
/// the `Engine`).
#[derive(Debug, Clone, Default)]
pub struct WasapiControl {
    inner: Arc<ControlInner>,
}

impl WasapiControl {
    /// Ask for exclusive mode (`true`) or shared mode (`false`) the next time the stream is
    /// opened, e.g. by `Engine::switch_device`. The currently open stream is not touched.
    pub fn set_exclusive(&self, exclusive: bool) {
        self.inner.exclusive.store(exclusive, Ordering::Release);
    }

    /// Whether exclusive mode is requested.
    #[must_use]
    pub fn exclusive(&self) -> bool {
        self.inner.exclusive.load(Ordering::Acquire)
    }

    /// How the most recent stream was opened, including why exclusive mode was not used.
    #[must_use]
    pub fn last_info(&self) -> Option<StreamInfo> {
        self.inner
            .last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The WASAPI [`AudioOutput`].
///
/// # Dropping a stream waits for its thread
/// The `OutputStream` contract says the callback has finished when `drop` returns, so dropping
/// the stream (which `Engine::switch_device`, `Engine::shutdown` and dropping the `Engine` do)
/// **joins the render thread**. The thread leaves its loop within one device period, so this is
/// quick unless a driver hangs inside `GetBuffer` / `ReleaseBuffer`; then the drop, and with it the
/// engine's, is delayed until the driver returns. That is the price of the guarantee.
///
/// # `open` can wait for the driver after its timeout
/// `open` gives up after a timeout only while the render thread is still building the device. Once
/// the thread has claimed the stream (it is priming the first buffer and starting the device) the
/// stream is committed, and `open` waits for that last step to report. That wait has no timeout of
/// its own: it is bounded only by the driver returning from `Start` / `ReleaseBuffer`.
#[derive(Debug)]
pub struct WasapiOutput {
    control: WasapiControl,
}

impl WasapiOutput {
    /// A new output (shared mode) and the handle to steer and observe it.
    #[must_use]
    pub fn new() -> (Self, WasapiControl) {
        let control = WasapiControl::default();
        (
            Self {
                control: control.clone(),
            },
            control,
        )
    }
}

struct ThreadSpec {
    device: Option<String>,
    rate: u32,
    exclusive: bool,
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    state: Arc<OpenState>,
}

struct Stream {
    stop: Arc<AtomicBool>,
    lost: Arc<AtomicBool>,
    info: StreamInfo,
    thread: Option<JoinHandle<Renderer>>,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasapiStream")
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
            // The render thread wakes with every device period (and at least every
            // `WAIT_TIMEOUT_MS`), sees `stop`, stops the stream and ends. When `join` returns the
            // callback has finished for good, which is the `OutputStream` drop contract. The
            // renderer is dropped here, on the caller's thread, not on the audio thread.
            let _ = thread.join();
        }
    }
}

impl AudioOutput for WasapiOutput {
    fn wants_exclusive(&self) -> bool {
        self.control.exclusive()
    }

    fn open(
        &mut self,
        request: &OpenRequest<'_>,
        renderer: Renderer,
    ) -> Result<Box<dyn OutputStream>, PlaybackError> {
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));
        let state = Arc::new(OpenState::new());
        let spec = ThreadSpec {
            device: request.device.map(str::to_owned),
            rate: request.sample_rate,
            exclusive: self.control.exclusive(),
            stop: Arc::clone(&stop),
            lost: Arc::clone(&lost),
            state: Arc::clone(&state),
        };
        let (opened_tx, opened_rx) = sync_channel::<Result<StreamInfo, String>>(1);
        let thread = std::thread::Builder::new()
            .name("tinwave-wasapi".into())
            .spawn(move || render_thread(&spec, renderer, &opened_tx))
            .map_err(|e| PlaybackError::Output(format!("cannot start the WASAPI thread: {e}")))?;
        let mut outcome = opened_rx.recv_timeout(OPEN_TIMEOUT);
        if outcome.is_err() {
            if state.abandon() {
                // The device is still being built. From here the thread owns nothing it may
                // render with: it drops what it built and ends without touching the renderer.
                stop.store(true, Ordering::Release);
                return Err(PlaybackError::Output(
                    "WASAPI: opening the stream timed out".into(),
                ));
            }
            // The thread claimed the stream just before the deadline: it is priming and will
            // report in a moment.
            outcome = opened_rx
                .recv()
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected);
        }
        match outcome {
            Ok(Ok(info)) => {
                *self
                    .control
                    .inner
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
                // The thread ended and returned the renderer; it is dropped here.
                let _ = thread.join();
                Err(PlaybackError::Output(format!("WASAPI: {message}")))
            }
            Err(_) => {
                let _ = thread.join();
                Err(PlaybackError::Output(
                    "WASAPI: the render thread ended unexpectedly".into(),
                ))
            }
        }
    }
}

fn render_thread(
    spec: &ThreadSpec,
    renderer: Renderer,
    opened: &std::sync::mpsc::SyncSender<Result<StreamInfo, String>>,
) -> Renderer {
    let com = initialize_mta().is_ok();
    let mmcss = os::MmcssGuard::enter();
    let realtime = mmcss.is_ok();
    let mmcss_failure = mmcss.as_ref().err().cloned();
    let renderer = run_render_thread(
        &spec.state,
        &spec.stop,
        &spec.lost,
        renderer,
        || open_device(spec, realtime, mmcss_failure),
        opened,
        WAIT_TIMEOUT_MS,
    );
    drop(mmcss);
    if com {
        deinitialize();
    }
    renderer
}

fn open_device(
    spec: &ThreadSpec,
    realtime_priority: bool,
    mmcss_failure: Option<String>,
) -> Result<DeviceSetup<WasapiDevice>, String> {
    let enumerator = DeviceEnumerator::new().map_err(|e| format!("WASAPI: {e}"))?;
    let (device, target) = match &spec.device {
        None => (
            enumerator
                .get_default_device(&Direction::Render)
                .map_err(|e| format!("WASAPI: no default output device: {e}"))?,
            FollowTarget::Default,
        ),
        Some(id) => (
            enumerator
                .get_device(id)
                .map_err(|e| format!("WASAPI: output device {id} not found: {e}"))?,
            FollowTarget::Device(id.clone()),
        ),
    };
    let device_id = device.get_id().map_err(|e| format!("WASAPI: {e}"))?;
    let device_name = device.get_friendlyname().unwrap_or_default();

    let mut notes: Vec<String> = Vec::new();
    let registration = match register_following(&enumerator, target, &device_id, &spec.lost) {
        Ok(r) => Some(r),
        Err(why) => {
            notes.push(format!("device changes are not followed: {why}"));
            None
        }
    };

    let rate = spec.rate as usize;
    let mut client = device
        .get_iaudioclient()
        .map_err(|e| format!("WASAPI: {e}"))?;
    let (default_period, min_period) = client
        .get_device_period()
        .map_err(|e| format!("WASAPI: {e}"))?;

    let outcome = resolve_mode(
        spec.exclusive,
        || open_exclusive(&mut client, rate, min_period),
        default_period,
    );
    if spec.exclusive && outcome.mode == StreamMode::Shared {
        // A client that failed to initialise cannot be reused.
        client = device
            .get_iaudioclient()
            .map_err(|e| format!("WASAPI: {e}"))?;
    }
    if outcome.mode == StreamMode::Shared {
        let wave = WaveFormat::new(32, 32, &SampleType::Float, rate, CHANNELS, None);
        client
            .initialize_client(
                &wave,
                &Direction::Render,
                &WasapiMode::EventsShared {
                    autoconvert: true,
                    buffer_duration_hns: default_period,
                },
            )
            .map_err(|e| format!("WASAPI: cannot open {device_name} in shared mode: {e}"))?;
    }
    if let Some(why) = &outcome.fallback {
        tracing::warn!(reason = %why, "exclusive mode refused, using shared mode");
    }
    if let Some(why) = mmcss_failure {
        tracing::warn!(reason = %why, "audio thread runs without MMCSS priority");
        notes.push(why);
    }
    notes.extend(outcome.fallback.clone());

    let events = client
        .set_get_eventhandle()
        .map_err(|e| format!("WASAPI: {e}"))?;
    let render = client
        .get_audiorenderclient()
        .map_err(|e| format!("WASAPI: {e}"))?;
    let buffer_frames = client
        .get_buffer_size()
        .map_err(|e| format!("WASAPI: {e}"))? as usize;
    let latency_hns = os::stream_latency_hns(client.as_iaudioclient());
    let hns_to_frames = |hns: i64| (hns.max(0) as u64 * u64::from(spec.rate) / 10_000_000) as u32;

    let info = StreamInfo {
        backend: "wasapi",
        device_id,
        device_name,
        mode: outcome.mode,
        format: outcome.format,
        period_frames: hns_to_frames(outcome.period_hns),
        buffer_frames: buffer_frames as u32,
        stream_latency_frames: hns_to_frames(latency_hns),
        realtime_priority,
        fallback: (!notes.is_empty()).then(|| notes.join("; ")),
    };
    Ok(DeviceSetup {
        stream_latency_frames: u64::from(info.stream_latency_frames),
        info,
        format: outcome.format,
        buffer_frames,
        device: WasapiDevice {
            client,
            render,
            events,
            _enumerator: enumerator,
            _registration: registration,
        },
    })
}

/// Probe the endpoint for an integer format and initialise the client exclusive and event driven.
fn open_exclusive(
    client: &mut AudioClient,
    rate: usize,
    min_period: i64,
) -> Result<(SampleFormat, i64), String> {
    let mut accepted: Option<WaveFormat> = None;
    let chosen = plan_exclusive(|format| {
        let (store, valid) = (format.bytes_per_sample() * 8, format.valid_bits());
        let want = WaveFormat::new(store, valid, &SampleType::Int, rate, CHANNELS, None);
        match client.is_supported_exclusive_with_quirks(&want) {
            Ok(wave) => {
                accepted = Some(wave);
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    })?;
    let wave = accepted.ok_or_else(|| "no format was accepted".to_string())?;
    let period = client
        .calculate_aligned_period_near(3 * min_period / 2, Some(128), &wave)
        .map_err(|e| format!("no usable period: {e}"))?;
    client
        .initialize_client(
            &wave,
            &Direction::Render,
            &WasapiMode::EventsExclusive { period_hns: period },
        )
        .map_err(|e| format!("the endpoint would not open {chosen:?}: {e}"))?;
    Ok((chosen, period))
}

/// Register the notification client that flips `lost` when the follower says to re-open.
fn register_following(
    enumerator: &DeviceEnumerator,
    target: FollowTarget,
    device_id: &str,
    lost: &Arc<AtomicBool>,
) -> Result<DeviceEventRegistration, String> {
    let follower = Arc::new(DeviceFollower::new(target, device_id));
    let judge = move |follower: &DeviceFollower, lost: &AtomicBool, event: DeviceEvent| {
        if follower.needs_reopen(&event) {
            lost.store(true, Ordering::Release);
        }
    };
    let mut callbacks = DeviceEventCallbacks::new();
    {
        let (follower, lost) = (Arc::clone(&follower), Arc::clone(lost));
        callbacks.set_default_device_callback(move |direction, role, new_id| {
            judge(
                &follower,
                &lost,
                DeviceEvent::DefaultChanged {
                    render: direction == Direction::Render,
                    general_role: matches!(role, Role::Console | Role::Multimedia),
                    new_id,
                },
            );
        });
    }
    {
        let (follower, lost) = (Arc::clone(&follower), Arc::clone(lost));
        callbacks.set_device_state_callback(move |id, state| {
            let state = match state {
                DeviceState::Active => DeviceStateChange::Active,
                DeviceState::Disabled => DeviceStateChange::Disabled,
                DeviceState::NotPresent => DeviceStateChange::NotPresent,
                DeviceState::Unplugged => DeviceStateChange::Unplugged,
            };
            judge(&follower, &lost, DeviceEvent::StateChanged { id, state });
        });
    }
    {
        let (follower, lost) = (Arc::clone(&follower), Arc::clone(lost));
        callbacks.set_device_removed_callback(move |id| {
            judge(&follower, &lost, DeviceEvent::Removed { id });
        });
    }
    enumerator
        .register_notification_callback(callbacks)
        .map_err(|e| e.to_string())
}

/// The WASAPI calls behind [`RenderDevice`].
struct WasapiDevice {
    client: AudioClient,
    render: AudioRenderClient,
    events: Handle,
    /// Kept alive for the endpoint objects and the notification registration.
    _enumerator: DeviceEnumerator,
    _registration: Option<DeviceEventRegistration>,
}

impl RenderDevice for WasapiDevice {
    fn wait(&mut self, timeout_ms: u32) -> Result<bool, DeviceLost> {
        // `wait_for_event` reports a timeout and a failed wait alike as an error; a device that
        // is really gone shows up in `is_alive` right after.
        Ok(self.events.wait_for_event(timeout_ms).is_ok())
    }

    fn available_frames(&mut self) -> Result<usize, DeviceLost> {
        self.client
            .get_available_space_in_frames()
            .map(|n| n as usize)
            .map_err(|_| DeviceLost)
    }

    fn write(&mut self, frames: usize, bytes: &[u8]) -> Result<(), DeviceLost> {
        self.render
            .write_to_device(frames, bytes, None)
            .map_err(|_| DeviceLost)
    }

    fn is_alive(&mut self) -> bool {
        self.client.get_current_padding().is_ok()
    }

    fn start(&mut self) -> Result<(), DeviceLost> {
        self.client.start_stream().map_err(|_| DeviceLost)
    }

    fn stop(&mut self) {
        let _ = self.client.stop_stream();
    }
}
