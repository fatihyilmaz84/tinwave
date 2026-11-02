//! The body of an output's render thread, independent of the operating system.
//!
//! An output's `open` starts a thread that builds the device (which can be slow: a Bluetooth
//! endpoint may take seconds to activate), then runs the callback loop. `open` cannot wait
//! forever, so it may give up. [`OpenState`] settles who owns the [`Renderer`] in that race: the
//! thread claims the stream with a compare-and-swap before its first `feed`, `open` abandons it
//! with the opposite swap on timeout, and exactly one of the two wins. A thread that lost never
//! touches the renderer, so it cannot acknowledge a flush or advance `consumed` on shared state
//! after the engine has moved on.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::SyncSender;

use super::convert::SampleFormat;
use super::info::StreamInfo;
use super::pump::{Pump, PumpStep, RenderDevice};
use crate::Renderer;

const OPENING: u8 = 0;
const RUNNING: u8 = 1;
const ABANDONED: u8 = 2;

/// Who owns a stream that is being opened: the opening thread, or nobody (abandoned).
#[derive(Debug)]
pub struct OpenState(AtomicU8);

impl Default for OpenState {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenState {
    /// A stream that is still being opened.
    #[must_use]
    pub fn new() -> Self {
        Self(AtomicU8::new(OPENING))
    }

    /// The render thread: take the stream. `false` means `open` gave up first and the thread
    /// must exit without touching the renderer.
    pub fn claim_run(&self) -> bool {
        self.0
            .compare_exchange(OPENING, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// `open`: give up after a timeout. `false` means the thread already took the stream, which
    /// is now running (or about to report), so `open` must collect its result after all.
    pub fn abandon(&self) -> bool {
        self.0
            .compare_exchange(OPENING, ABANDONED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

/// A device the render thread has built, ready to be primed and started.
#[derive(Debug)]
pub struct DeviceSetup<D> {
    /// The device calls.
    pub device: D,
    /// What the stream looks like (reported to `open` once the stream runs).
    pub info: StreamInfo,
    /// Format written to the device.
    pub format: SampleFormat,
    /// Frames in the device buffer.
    pub buffer_frames: usize,
    /// OS latency beyond the buffer, in frames.
    pub stream_latency_frames: u64,
}

/// Build the device with `open`, claim the stream, prime and start it, then pump until `stop`.
/// Reports the outcome on `opened` exactly once (unless abandoned, when nobody is listening).
/// Returns the renderer for the caller to drop on a thread of its choosing.
pub fn run_render_thread<D: RenderDevice>(
    state: &OpenState,
    stop: &AtomicBool,
    lost: &AtomicBool,
    renderer: Renderer,
    open: impl FnOnce() -> Result<DeviceSetup<D>, String>,
    opened: &SyncSender<Result<StreamInfo, String>>,
    wait_timeout_ms: u32,
) -> Renderer {
    let setup = match open() {
        Ok(setup) => setup,
        Err(message) => {
            let _ = opened.send(Err(message));
            return renderer;
        }
    };
    // From here on the renderer may be rendered. Before this point `open` can still give up, and
    // then nothing below runs: the abandoned thread only drops what it built.
    if !state.claim_run() {
        return renderer;
    }
    let DeviceSetup {
        mut device,
        info,
        format,
        buffer_frames,
        stream_latency_frames,
    } = setup;
    let mut pump = Pump::new(renderer, format, buffer_frames, stream_latency_frames);
    // Prime the buffer with real audio, then start: the first period already plays.
    let primed = matches!(
        pump.feed(&mut device),
        PumpStep::Fed { .. } | PumpStep::Idle
    ) && device.start().is_ok();
    if primed {
        let _ = opened.send(Ok(info));
        while !stop.load(Ordering::Acquire) {
            if pump.step(&mut device, wait_timeout_ms) == PumpStep::Lost {
                lost.store(true, Ordering::Release);
                break;
            }
        }
        device.stop();
    } else {
        let _ = opened.send(Err(
            "the device refused the first buffer or could not start".into(),
        ));
    }
    pump.into_renderer()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering::{Acquire, Release};
    use std::sync::mpsc::{channel, sync_channel};
    use std::sync::Arc;

    use rtrb::RingBuffer;

    use super::*;
    use crate::output::pump::DeviceLost;
    use crate::shared::Shared;

    #[derive(Default)]
    struct Probe {
        writes: AtomicU8,
        started: AtomicBool,
    }

    struct FakeDevice {
        probe: Arc<Probe>,
    }

    impl RenderDevice for FakeDevice {
        fn wait(&mut self, _timeout_ms: u32) -> Result<bool, DeviceLost> {
            Ok(true)
        }
        fn available_frames(&mut self) -> Result<usize, DeviceLost> {
            Ok(480)
        }
        fn write(&mut self, _frames: usize, _bytes: &[u8]) -> Result<(), DeviceLost> {
            self.probe.writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn is_alive(&mut self) -> bool {
            true
        }
        fn start(&mut self) -> Result<(), DeviceLost> {
            self.probe.started.store(true, Ordering::Release);
            Ok(())
        }
        fn stop(&mut self) {}
    }

    fn setup(probe: &Arc<Probe>) -> DeviceSetup<FakeDevice> {
        DeviceSetup {
            device: FakeDevice {
                probe: Arc::clone(probe),
            },
            info: StreamInfo::unknown(),
            format: SampleFormat::F32,
            buffer_frames: 480,
            stream_latency_frames: 0,
        }
    }

    /// A renderer with audio waiting in its ring and a flush request pending on the shared state.
    fn renderer_with_audio() -> (Arc<Shared>, rtrb::Producer<f32>, Renderer) {
        let sh = Shared::new(1.0);
        sh.eos.store(false, Release);
        sh.flush_req.store(1, Release);
        let (mut prod, cons) = RingBuffer::<f32>::new(9_600);
        let _ = prod.push_partial_slice(&[0.25; 4_800]);
        (Arc::clone(&sh), prod, Renderer::new(cons, sh))
    }

    #[test]
    fn a_stream_abandoned_while_it_was_still_opening_never_renders() {
        let (sh, _producer, renderer) = renderer_with_audio();
        let probe = Arc::new(Probe::default());
        let state = Arc::new(OpenState::new());
        let (stop, lost) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let (opened_tx, opened_rx) = sync_channel(1);
        let (gate_tx, gate_rx) = channel::<()>();
        let thread = {
            let (state, stop, lost, probe) = (
                Arc::clone(&state),
                Arc::clone(&stop),
                Arc::clone(&lost),
                Arc::clone(&probe),
            );
            std::thread::spawn(move || {
                run_render_thread(
                    &state,
                    &stop,
                    &lost,
                    renderer,
                    || {
                        // A slow open: the device takes its time (a Bluetooth endpoint waking up).
                        let _ = gate_rx.recv();
                        Ok(setup(&probe))
                    },
                    &opened_tx,
                    10,
                )
            })
        };

        // `open` times out and gives up while the device is still being built.
        assert!(state.abandon(), "the thread had not claimed the stream yet");
        stop.store(true, Ordering::Release);
        // Only now does the slow open finish.
        gate_tx.send(()).expect("thread is waiting");
        let mut renderer = thread.join().expect("thread ends");

        assert_eq!(
            probe.writes.load(Ordering::Relaxed),
            0,
            "nothing was written"
        );
        assert!(!probe.started.load(Acquire), "the device was never started");
        assert_eq!(sh.consumed.load(Acquire), 0, "no audio was consumed");
        assert_eq!(
            sh.flush_ack.load(Acquire),
            0,
            "the flush request was not acknowledged"
        );
        assert_eq!(renderer.buffered_frames(), 2_400, "the ring is untouched");
        assert!(
            opened_rx.try_recv().is_err(),
            "nothing was reported to a caller that left"
        );
        assert!(!lost.load(Ordering::Acquire));
        // And the engine can still use the renderer it got back: it behaves as before.
        let mut out = [0.0f32; 480];
        renderer.render(&mut out, 0);
        assert_eq!(sh.flush_ack.load(Acquire), 1);
    }

    #[test]
    fn a_stream_that_claimed_first_runs_and_cannot_be_abandoned() {
        let (sh, _producer, renderer) = renderer_with_audio();
        let probe = Arc::new(Probe::default());
        let state = Arc::new(OpenState::new());
        let (stop, lost) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let (opened_tx, opened_rx) = sync_channel(1);
        let thread = {
            let (state, stop, lost, probe) = (
                Arc::clone(&state),
                Arc::clone(&stop),
                Arc::clone(&lost),
                Arc::clone(&probe),
            );
            std::thread::spawn(move || {
                run_render_thread(
                    &state,
                    &stop,
                    &lost,
                    renderer,
                    || Ok(setup(&probe)),
                    &opened_tx,
                    10,
                )
            })
        };
        let info = opened_rx.recv().expect("reported").expect("opened");
        assert_eq!(info.backend, "unknown");
        assert!(!state.abandon(), "too late: the thread owns the stream");
        stop.store(true, Ordering::Release);
        let _renderer = thread.join().expect("thread ends");
        assert!(probe.started.load(Acquire));
        assert!(probe.writes.load(Ordering::Relaxed) >= 1);
        assert_eq!(
            sh.flush_ack.load(Acquire),
            1,
            "the running stream serviced the flush"
        );
    }

    #[test]
    fn a_failed_open_is_reported_and_the_renderer_comes_back_untouched() {
        let (sh, _producer, renderer) = renderer_with_audio();
        let state = OpenState::new();
        let (stop, lost) = (AtomicBool::new(false), AtomicBool::new(false));
        let (opened_tx, opened_rx) = sync_channel(1);
        let renderer = run_render_thread::<FakeDevice>(
            &state,
            &stop,
            &lost,
            renderer,
            || Err("no such device".into()),
            &opened_tx,
            10,
        );
        assert_eq!(
            opened_rx.recv().expect("reported"),
            Err("no such device".into())
        );
        assert_eq!(renderer.buffered_frames(), 2_400);
        assert_eq!(sh.consumed.load(Acquire), 0);
    }
}
