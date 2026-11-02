//! The audio output-device and system-volume providers (ADR 0004).
//!
//! One provider thread owns the OS notification objects and a small state machine
//! ([`engine`]). It feeds two views of one cache, [`OutputDeviceProvider`] (T1) and
//! [`SystemVolumeProvider`] (T0), and emits three signals:
//!
//! | Signal | Tier | When |
//! |---|---|---|
//! | `SystemVolumeChanged` | T0 | the listener's volume gesture ends (400 ms trailing window) |
//! | `MuteToggled` | T0 | the endpoint's mute changed (the scalar did not) |
//! | `OutputDeviceChanged` | T1 | the followed endpoint changed |
//!
//! Playback never waits for any of this: `snapshot()` reads an `ArcSwap` cache, the OS callbacks
//! only copy and `try_send`, and the OS is only called from the provider thread.
//!
//! On Windows the OS side is MMDevice/WASAPI notifications (`win/`). Elsewhere
//! [`AudioMonitor::start`] succeeds without a thread, every field stays unknown and
//! `capability()` is `Unsupported`.

// The thread, engine and fake-backend tests compile everywhere; off Windows only tests use them.
#![cfg_attr(not(windows), allow(dead_code))]

mod backend;
mod classify;
mod engine;
#[cfg(test)]
mod fake;
mod identity;
mod runtime;
mod stats;

#[cfg(windows)]
mod win;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use tinwave_core::Tier;
use tinwave_signals::{ElapsedDs, EventContext, Section, SignalBus};

pub use backend::EVENT_CONTEXT_GUID;
pub use classify::{
    classify, form_factor, form_factor_byte, transport_from_connected_id, JackDescription,
    Transport,
};
pub use identity::{DeviceSecret, DEVICE_SECRET_LEN};
pub use stats::AudioStats;

use crate::error::ContextError;
use crate::provider::{Capability, ContextFields, ContextProvider, ProviderId};
use stats::StatsCells;

/// Why the audio providers do nothing on this OS.
#[cfg(not(windows))]
const UNSUPPORTED_REASON: &str = "audio output and system volume are only implemented for Windows";

/// Which endpoint the volume notification follows.
///
/// `Debug` hides the endpoint id: it identifies a piece of hardware and must not reach logs.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum OutputTarget {
    /// The default render device for the multimedia role; re-binds when the default changes.
    #[default]
    SystemDefault,
    /// A device the listener chose in Tinwave: the OS endpoint id (held in memory only).
    Device(String),
}

impl fmt::Debug for OutputTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SystemDefault => f.write_str("SystemDefault"),
            Self::Device(_) => f.write_str("Device(<redacted>)"),
        }
    }
}

/// What is playing right now: the columns every emitted event needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackSnapshot {
    /// Session, recording, source, position and context snapshot of the event.
    pub event: EventContext,
    /// Section playing now.
    pub section: Section,
    /// Time since the track started.
    pub since_track_start: ElapsedDs,
    /// Time since the current section started.
    pub since_section_start: ElapsedDs,
}

/// Tells the provider whether Tinwave is playing, and the session to attribute events to.
///
/// "While Tinwave plays" is the consumer's gate (ADR 0004 section 4): return `None` when nothing
/// plays and the provider keeps tracking (so `from` stays right) but emits nothing.
/// Implementations must not block: they run on the provider thread, not the audio thread, but a
/// slow answer delays gesture flushes.
pub trait PlaybackContextSource: Send + Sync {
    /// The current playback, or `None` when nothing is playing.
    fn current(&self) -> Option<PlaybackSnapshot>;
}

/// Everything [`AudioMonitor::start`] needs.
pub struct AudioMonitorConfig {
    /// Where events go. The provider thread uses its own clone (and its own coalescer).
    pub bus: SignalBus,
    /// The per-install salt of `device_hash`. Generate with [`DeviceSecret::generate`], keep it
    /// in the keychain, and pass the same bytes on every start.
    pub secret: DeviceSecret,
    /// Playback state, for the "while playing" gate and the event columns.
    pub playback: Arc<dyn PlaybackContextSource>,
    /// Which endpoint to follow.
    pub target: OutputTarget,
}

impl fmt::Debug for AudioMonitorConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioMonitorConfig")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// The running audio providers. Dropping it stops the thread and removes the OS registrations.
#[derive(Debug)]
pub struct AudioMonitor {
    cache: Arc<ArcSwap<ContextFields>>,
    stats: Arc<StatsCells>,
    thread: Option<runtime::ProviderThread>,
}

impl AudioMonitor {
    /// Starts the provider thread (Windows) or reports `Unsupported` (elsewhere).
    ///
    /// On Windows a missing audio service, a session without audio or a failed bind is **not**
    /// an error: the fields stay unknown and [`AudioStats::degraded`] says why nothing arrives
    /// (ADR 0004 section 7).
    ///
    /// # Errors
    /// [`ContextError::ThreadSpawn`] if the OS refuses to start a thread.
    pub fn start(config: AudioMonitorConfig) -> Result<Self, ContextError> {
        let cache = Arc::new(ArcSwap::from_pointee(ContextFields::default()));
        let stats = Arc::new(StatsCells::default());
        let thread = Self::spawn(config, &cache, &stats)?;
        Ok(Self {
            cache,
            stats,
            thread,
        })
    }

    #[cfg(windows)]
    fn spawn(
        config: AudioMonitorConfig,
        cache: &Arc<ArcSwap<ContextFields>>,
        stats: &Arc<StatsCells>,
    ) -> Result<Option<runtime::ProviderThread>, ContextError> {
        win::spawn(config, cache.clone(), stats.clone()).map(Some)
    }

    #[cfg(not(windows))]
    fn spawn(
        _config: AudioMonitorConfig,
        _cache: &Arc<ArcSwap<ContextFields>>,
        _stats: &Arc<StatsCells>,
    ) -> Result<Option<runtime::ProviderThread>, ContextError> {
        Ok(None)
    }

    /// What the audio providers can do on this OS: `Supported` on Windows, `Unsupported` elsewhere.
    #[must_use]
    pub fn capability() -> Capability {
        #[cfg(windows)]
        {
            Capability::Supported
        }
        #[cfg(not(windows))]
        {
            Capability::Unsupported(UNSUPPORTED_REASON)
        }
    }

    /// The output-device provider (T1).
    #[must_use]
    pub fn output_device(&self) -> OutputDeviceProvider {
        OutputDeviceProvider {
            cache: self.cache.clone(),
        }
    }

    /// The system-volume provider (T0).
    #[must_use]
    pub fn system_volume(&self) -> SystemVolumeProvider {
        SystemVolumeProvider {
            cache: self.cache.clone(),
        }
    }

    /// Follows another endpoint. Emits `OutputDeviceChanged` if it differs from the current one.
    ///
    /// # Errors
    /// [`ContextError::ProviderStopped`] if the thread has ended (or never ran, off Windows);
    /// [`ContextError::CommandQueueFull`] if it is far behind: try again.
    pub fn set_target(&self, target: OutputTarget) -> Result<(), ContextError> {
        match &self.thread {
            Some(thread) => thread.set_target(target),
            None => Err(ContextError::ProviderStopped),
        }
    }

    /// Diagnostic counters.
    #[must_use]
    pub fn stats(&self) -> AudioStats {
        self.stats.snapshot()
    }
}

/// The output-device provider: class, transport and form factor of the followed endpoint.
#[derive(Debug, Clone)]
pub struct OutputDeviceProvider {
    cache: Arc<ArcSwap<ContextFields>>,
}

impl ContextProvider for OutputDeviceProvider {
    fn id(&self) -> ProviderId {
        ProviderId::OutputDevice
    }

    fn tier(&self) -> Tier {
        Tier::T1
    }

    fn max_age(&self) -> Duration {
        Duration::MAX
    }

    fn snapshot(&self, out: &mut ContextFields) {
        out.output_device = self.cache.load().output_device;
    }

    fn capability(&self) -> Capability {
        AudioMonitor::capability()
    }
}

/// The system-volume provider: slider position, mute and endpoint gain of the followed endpoint.
#[derive(Debug, Clone)]
pub struct SystemVolumeProvider {
    cache: Arc<ArcSwap<ContextFields>>,
}

impl ContextProvider for SystemVolumeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::SystemVolume
    }

    fn tier(&self) -> Tier {
        Tier::T0
    }

    fn max_age(&self) -> Duration {
        Duration::MAX
    }

    fn snapshot(&self, out: &mut ContextFields) {
        out.system_volume = self.cache.load().system_volume;
    }

    fn capability(&self) -> Capability {
        AudioMonitor::capability()
    }
}

#[cfg(test)]
#[path = "monitor_tests.rs"]
mod tests;
