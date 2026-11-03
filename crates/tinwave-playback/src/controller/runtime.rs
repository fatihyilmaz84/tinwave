//! The controller on its own thread.
//!
//! The thread owns the [`Controller`] (the engine is driven from one thread) and loops on one
//! blocking receive with the poll interval as timeout. After every command and every timeout it
//! calls [`Controller::poll`], so the cadence holds under a stream of commands too, and nothing
//! here reads a clock: the timeout is scheduling, not data.

use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use tinwave_signals::{InputSource, RepeatMode, View};

use super::types::{
    ControllerConfig, ControllerDiagnostics, LoadRequest, PlaybackStatus, QueueEditRequest,
    QueueEntry,
};
use super::Controller;
use crate::{AudioOutput, PlaybackError};

/// Commands waiting for the controller thread before calls fail with [`PlaybackError::Busy`].
const COMMAND_QUEUE: usize = 64;

type Reply = SyncSender<Result<(), PlaybackError>>;

enum Command {
    Load(LoadRequest),
    Append(Vec<QueueEntry>),
    QueueLater(Vec<QueueEntry>, View, InputSource),
    EditQueue(QueueEditRequest),
    Stop,
    Seek(u64, InputSource),
    Pause(InputSource),
    Resume(InputSource),
    Volume(f32, InputSource),
    Repeat(RepeatMode),
    SwitchDevice(Option<String>),
}

enum Message {
    Run(Command, Reply),
    Shutdown,
}

/// The app's handle on a running controller. Cheap to share by reference (`Send + Sync`).
///
/// Every command returns once the controller thread has carried it out, with the engine's own
/// answer. The wait is the time of one engine call and never includes the signal bus, the
/// database, analysis or the network. Engine calls take microseconds, except
/// [`switch_device`](Self::switch_device), which opens the output synchronously (100 ms or more
/// for Bluetooth or exclusive WASAPI): call it off the UI thread.
///
/// Dropping the handle shuts the controller down like [`shutdown`](Self::shutdown): playback is
/// stopped, pending signals are emitted and the session ends, but the decoder thread is **not**
/// waited for, so a read stuck on an unreachable network share cannot hang application exit.
pub struct ControllerHandle {
    tx: SyncSender<Message>,
    status: Arc<Mutex<PlaybackStatus>>,
    diagnostics: Arc<Mutex<ControllerDiagnostics>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for ControllerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerHandle")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl ControllerHandle {
    /// Starts the engine and the controller thread.
    ///
    /// # Errors
    /// As [`Controller::start`]; [`PlaybackError::Output`] when the thread cannot be spawned.
    pub fn spawn(
        config: ControllerConfig,
        output: Box<dyn AudioOutput>,
    ) -> Result<Self, PlaybackError> {
        let interval = config.poll_interval;
        let mut controller = Controller::start(config, output)?;
        let status = Arc::new(Mutex::new(controller.status()));
        let (tx, rx) = sync_channel(COMMAND_QUEUE);
        let diagnostics = Arc::new(Mutex::new(controller.diagnostics()));
        let shared = Shared {
            status: Arc::clone(&status),
            diagnostics: Arc::clone(&diagnostics),
        };
        let thread = std::thread::Builder::new()
            .name("tinwave-controller".into())
            .spawn(move || {
                run(&mut controller, &rx, interval, &shared);
                // Never joins the decoder: exit must not wait for a stuck read.
                controller.close();
            })
            .map_err(|e| PlaybackError::Output(format!("cannot start controller thread: {e}")))?;
        Ok(Self {
            tx,
            status,
            diagnostics,
            thread: Some(thread),
        })
    }

    /// Replace the queue and start playing one of its entries. See [`Controller::load`].
    ///
    /// # Errors
    /// As [`Controller::load`], or [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn load(&self, request: LoadRequest) -> Result<(), PlaybackError> {
        self.call(Command::Load(request))
    }

    /// Add entries after the last one. See [`Controller::append`].
    ///
    /// # Errors
    /// As [`load`](Self::load).
    pub fn append(&self, entries: Vec<QueueEntry>) -> Result<(), PlaybackError> {
        self.call(Command::Append(entries))
    }

    /// The listener added entries to the end of the queue. See [`Controller::queue_later`].
    ///
    /// # Errors
    /// As [`load`](Self::load).
    pub fn queue_later(
        &self,
        entries: Vec<QueueEntry>,
        from_view: View,
        input: InputSource,
    ) -> Result<(), PlaybackError> {
        self.call(Command::QueueLater(entries, from_view, input))
    }

    /// Play next, reorder or remove without interrupting the playing track. See
    /// [`Controller::edit_queue`].
    ///
    /// # Errors
    /// As [`Controller::edit_queue`], or [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn edit_queue(&self, request: QueueEditRequest) -> Result<(), PlaybackError> {
        self.call(Command::EditQueue(request))
    }

    /// Stop playback and clear the queue.
    ///
    /// # Errors
    /// As [`load`](Self::load).
    pub fn stop(&self) -> Result<(), PlaybackError> {
        self.call(Command::Stop)
    }

    /// Seek to `position_ms`. See [`Controller::seek_ms`].
    ///
    /// # Errors
    /// As [`load`](Self::load).
    pub fn seek_ms(&self, position_ms: u64, input: InputSource) -> Result<(), PlaybackError> {
        self.call(Command::Seek(position_ms, input))
    }

    /// Pause.
    ///
    /// # Errors
    /// [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn pause(&self, input: InputSource) -> Result<(), PlaybackError> {
        self.call(Command::Pause(input))
    }

    /// Resume.
    ///
    /// # Errors
    /// [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn resume(&self, input: InputSource) -> Result<(), PlaybackError> {
        self.call(Command::Resume(input))
    }

    /// Set the software volume. See [`Controller::set_volume`].
    ///
    /// # Errors
    /// As [`load`](Self::load).
    pub fn set_volume(&self, volume_linear: f32, input: InputSource) -> Result<(), PlaybackError> {
        self.call(Command::Volume(volume_linear, input))
    }

    /// Record the repeat choice. See [`Controller::set_repeat_mode`].
    ///
    /// # Errors
    /// [`PlaybackError::Busy`] / [`PlaybackError::EngineStopped`].
    pub fn set_repeat_mode(&self, mode: RepeatMode) -> Result<(), PlaybackError> {
        self.call(Command::Repeat(mode))
    }

    /// Move to another output device. See [`Controller::switch_device`].
    ///
    /// # Errors
    /// As [`load`](Self::load).
    pub fn switch_device(&self, device: Option<String>) -> Result<(), PlaybackError> {
        self.call(Command::SwitchDevice(device))
    }

    /// The state as of the controller thread's last step (at most one poll interval old).
    #[must_use]
    pub fn status(&self) -> PlaybackStatus {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Position, pending command and sink losses as of the controller thread's last step.
    #[must_use]
    pub fn diagnostics(&self) -> ControllerDiagnostics {
        *self
            .diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops playback, ends the session and joins the controller thread (not the decoder).
    pub fn shutdown(mut self) {
        self.stop_thread();
    }

    fn call(&self, command: Command) -> Result<(), PlaybackError> {
        let (reply, answer) = sync_channel(1);
        self.tx
            .try_send(Message::Run(command, reply))
            .map_err(|e| match e {
                TrySendError::Full(_) => PlaybackError::Busy,
                TrySendError::Disconnected(_) => PlaybackError::EngineStopped,
            })?;
        answer.recv().map_err(|_| PlaybackError::EngineStopped)?
    }

    fn stop_thread(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        // The thread also stops when the sender is gone, so a full queue is no obstacle: the
        // wake-up is only to avoid waiting out one poll interval.
        let _ = self.tx.try_send(Message::Shutdown);
        if thread.join().is_err() {
            tracing::error!("the controller thread panicked");
        }
    }
}

impl Drop for ControllerHandle {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

/// What the controller thread publishes after every step.
struct Shared {
    status: Arc<Mutex<PlaybackStatus>>,
    diagnostics: Arc<Mutex<ControllerDiagnostics>>,
}

fn run(controller: &mut Controller, rx: &Receiver<Message>, interval: Duration, shared: &Shared) {
    loop {
        let reply = match rx.recv_timeout(interval) {
            Ok(Message::Run(command, reply)) => Some((execute(controller, command), reply)),
            Ok(Message::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => None,
        };
        controller.poll();
        *shared.status.lock().unwrap_or_else(PoisonError::into_inner) = controller.status();
        *shared
            .diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = controller.diagnostics();
        if let Some((result, reply)) = reply {
            // The caller may have given up waiting; the command has been carried out anyway.
            let _ = reply.send(result);
        }
    }
}

fn execute(controller: &mut Controller, command: Command) -> Result<(), PlaybackError> {
    match command {
        Command::Load(request) => controller.load(request),
        Command::Append(entries) => controller.append(entries),
        Command::QueueLater(entries, view, input) => controller.queue_later(entries, view, input),
        Command::EditQueue(request) => controller.edit_queue(request),
        Command::Stop => controller.stop(),
        Command::Seek(ms, input) => controller.seek_ms(ms, input),
        Command::Pause(input) => {
            controller.pause(input);
            Ok(())
        }
        Command::Resume(input) => {
            controller.resume(input);
            Ok(())
        }
        Command::Volume(volume, input) => controller.set_volume(volume, input),
        Command::Repeat(mode) => {
            controller.set_repeat_mode(mode);
            Ok(())
        }
        Command::SwitchDevice(device) => controller.switch_device(device),
    }
}
