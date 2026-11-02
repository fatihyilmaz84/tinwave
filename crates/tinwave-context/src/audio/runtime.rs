//! The provider thread: owns the backend and the engine, and waits for OS events, commands and
//! the next gesture deadline. Platform independent; the Windows backend and the test fakes both
//! run on it.

use std::sync::Arc;
use std::thread::JoinHandle;

use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Select, Sender, TrySendError};
use tracing::warn;

use super::backend::{Backend, RawEvent};
use super::engine::Engine;
use super::stats::StatsCells;
use super::{AudioMonitorConfig, OutputTarget};
use crate::error::ContextError;
use crate::provider::ContextFields;

/// Capacity of the OS-event queue. A volume drag is ~100 events/s; this is seconds of backlog.
const EVENT_QUEUE: usize = 1024;
/// Capacity of the command queue (target changes are rare).
const COMMAND_QUEUE: usize = 8;

/// What the OS callbacks get: a bounded, non-blocking way to hand events to the thread.
#[derive(Debug, Clone)]
pub(crate) struct EventSender {
    tx: Sender<RawEvent>,
    /// "A device event was lost: re-query the OS." Capacity 1: a pending request is enough.
    resync: Sender<()>,
    stats: Arc<StatsCells>,
}

impl EventSender {
    /// Never blocks. A full queue drops the event and counts it. Losing a *device* event
    /// (default change, state change, removal) would leave the binding stale until the next
    /// one, so it also asks the thread to re-query the OS once the backlog is drained; losing
    /// a volume step is harmless (the next one carries the absolute value).
    pub(crate) fn send(&self, event: RawEvent) {
        let is_device_event = !matches!(event, RawEvent::Volume { .. });
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                StatsCells::bump(&self.stats.callback_dropped);
                if is_device_event {
                    // `Full` means a request is already pending: nothing to add.
                    let _ = self.resync.try_send(());
                }
            }
        }
    }

    pub(crate) fn count_property_noise(&self) {
        StatsCells::bump(&self.stats.property_noise);
    }
}

/// What the thread hands the backend factory.
#[derive(Debug)]
pub(crate) struct BackendContext {
    pub(crate) events: EventSender,
    pub(crate) bus: tinwave_signals::SignalBus,
}

#[derive(Debug)]
enum Command {
    SetTarget(OutputTarget),
}

/// Handle to the running thread. Dropping it stops the thread and joins it.
#[derive(Debug)]
pub(crate) struct ProviderThread {
    commands: Option<Sender<Command>>,
    join: Option<JoinHandle<()>>,
}

impl ProviderThread {
    pub(crate) fn set_target(&self, target: OutputTarget) -> Result<(), ContextError> {
        let Some(commands) = &self.commands else {
            return Err(ContextError::ProviderStopped);
        };
        commands
            .try_send(Command::SetTarget(target))
            .map_err(|e| match e {
                TrySendError::Full(_) => ContextError::CommandQueueFull,
                TrySendError::Disconnected(_) => ContextError::ProviderStopped,
            })
    }
}

impl Drop for ProviderThread {
    fn drop(&mut self) {
        // Closing the command channel is the shutdown signal.
        drop(self.commands.take());
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                warn!("the audio provider thread panicked");
            }
        }
    }
}

/// Starts the provider thread. `factory` runs **on the new thread** and creates the backend
/// there (COM objects must be created on the thread that owns them); if it fails, the thread
/// keeps the fields unknown, marks the stats `degraded` and waits for shutdown.
pub(crate) fn spawn<B, F>(
    config: AudioMonitorConfig,
    cache: Arc<ArcSwap<ContextFields>>,
    stats: Arc<StatsCells>,
    factory: F,
) -> Result<ProviderThread, ContextError>
where
    B: Backend,
    F: FnOnce(BackendContext) -> Result<B, String> + Send + 'static,
{
    spawn_with_queue(config, cache, stats, EVENT_QUEUE, factory)
}

/// [`spawn`] with an explicit OS-event queue capacity (tests shrink it to force a drop).
pub(crate) fn spawn_with_queue<B, F>(
    config: AudioMonitorConfig,
    cache: Arc<ArcSwap<ContextFields>>,
    stats: Arc<StatsCells>,
    event_capacity: usize,
    factory: F,
) -> Result<ProviderThread, ContextError>
where
    B: Backend,
    F: FnOnce(BackendContext) -> Result<B, String> + Send + 'static,
{
    let (event_tx, event_rx) = crossbeam_channel::bounded(event_capacity);
    let (resync_tx, resync_rx) = crossbeam_channel::bounded(1);
    let (command_tx, command_rx) = crossbeam_channel::bounded(COMMAND_QUEUE);
    let events = EventSender {
        tx: event_tx,
        resync: resync_tx,
        stats: stats.clone(),
    };
    let join = std::thread::Builder::new()
        .name("tinwave-audio-context".into())
        .spawn(move || {
            let context = BackendContext {
                events,
                bus: config.bus.clone(),
            };
            let backend = match factory(context) {
                Ok(backend) => backend,
                Err(error) => {
                    warn!(%error, "audio device and volume tracking is unavailable");
                    stats
                        .degraded
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    // Fields stay unknown; just wait until the owner goes away.
                    while command_rx.recv().is_ok() {}
                    return;
                }
            };
            let mut engine = Engine::new(
                backend,
                config.bus,
                config.playback,
                config.secret,
                config.target,
                cache,
                stats,
            );
            run(&mut engine, &event_rx, &resync_rx, &command_rx);
            engine.shutdown();
            // `engine` (and with it the backend and its COM guard) drops here, on this thread.
        })
        .map_err(|source| ContextError::ThreadSpawn {
            what: "audio context",
            source,
        })?;
    Ok(ProviderThread {
        commands: Some(command_tx),
        join: Some(join),
    })
}

fn run<B: Backend>(
    engine: &mut Engine<B>,
    events: &Receiver<RawEvent>,
    resync: &Receiver<()>,
    commands: &Receiver<Command>,
) {
    engine.start();
    loop {
        let mut select = Select::new();
        let event_op = select.recv(events);
        let resync_op = select.recv(resync);
        let command_op = select.recv(commands);
        let ready = match engine.wait_hint() {
            Some(timeout) => select.select_timeout(timeout).ok(),
            None => Some(select.select()),
        };
        match ready {
            Some(op) if op.index() == event_op => {
                match op.recv(events) {
                    Ok(event) => {
                        engine.handle(event);
                        // The whole backlog (a drag) before looking at the clock again.
                        while let Ok(event) = events.try_recv() {
                            engine.handle(event);
                        }
                    }
                    // Every sender is gone, so the OS can no longer reach us (the backend
                    // holds one while it lives). Stop instead of spinning on a dead channel.
                    Err(_) => {
                        warn!("the OS event channel closed; stopping the audio provider");
                        return;
                    }
                }
            }
            Some(op) if op.index() == resync_op => {
                if op.recv(resync).is_err() {
                    warn!("the OS event channel closed; stopping the audio provider");
                    return;
                }
                // Handle what is queued first: it may already say what was lost.
                while let Ok(event) = events.try_recv() {
                    engine.handle(event);
                }
                engine.resync();
            }
            Some(op) if op.index() == command_op => match op.recv(commands) {
                Ok(Command::SetTarget(target)) => engine.set_target(target),
                // Every sender dropped: the owner is shutting down.
                Err(_) => return,
            },
            // Timeout (a gesture is due) or an index we did not register.
            Some(_) | None => {}
        }
        engine.settle();
    }
}
