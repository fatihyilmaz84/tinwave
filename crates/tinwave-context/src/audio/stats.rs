//! Counters for the audio provider: how much was heard, dropped and ignored.
//!
//! High-volume noise (property changes) is counted here instead of being logged or sent as
//! signals; the numbers are for diagnostics and tests, not for the signal database.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A point-in-time copy of the provider's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioStats {
    /// The OS backend could not be started: every field stays `Unknown` (ADR 0004 section 7).
    pub degraded: bool,
    /// Volume notifications handled for the bound endpoint.
    pub volume_events: u64,
    /// Volume notifications caused by Tinwave itself (its event-context GUID): not signals.
    pub self_changes: u64,
    /// Notifications from an endpoint we are no longer bound to.
    pub stale_events: u64,
    /// `OnPropertyValueChanged` calls: counted in the callback, never forwarded.
    pub property_noise: u64,
    /// Callback events lost because the provider thread was far behind.
    pub callback_dropped: u64,
    /// Default-device callbacks ignored: other flow or role, or repeats of the bound id.
    pub default_repeats: u64,
    /// Times the volume binding moved to another endpoint, or to none.
    pub rebinds: u64,
    /// Times binding to an endpoint failed.
    pub bind_failures: u64,
    /// Changes that were tracked but not emitted because nothing was playing.
    pub suppressed_not_playing: u64,
    /// The largest delay between a callback and the provider thread handling it.
    pub max_lag_ms: u64,
}

/// Shared atomic cells behind [`AudioStats`]; written by the callbacks and the provider thread.
#[derive(Debug, Default)]
pub(crate) struct StatsCells {
    pub(crate) degraded: AtomicBool,
    pub(crate) volume_events: AtomicU64,
    pub(crate) self_changes: AtomicU64,
    pub(crate) stale_events: AtomicU64,
    pub(crate) property_noise: AtomicU64,
    pub(crate) callback_dropped: AtomicU64,
    pub(crate) default_repeats: AtomicU64,
    pub(crate) rebinds: AtomicU64,
    pub(crate) bind_failures: AtomicU64,
    pub(crate) suppressed_not_playing: AtomicU64,
    pub(crate) max_lag_ms: AtomicU64,
}

impl StatsCells {
    pub(crate) fn bump(cell: &AtomicU64) {
        cell.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> AudioStats {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        AudioStats {
            degraded: self.degraded.load(Ordering::Relaxed),
            volume_events: get(&self.volume_events),
            self_changes: get(&self.self_changes),
            stale_events: get(&self.stale_events),
            property_noise: get(&self.property_noise),
            callback_dropped: get(&self.callback_dropped),
            default_repeats: get(&self.default_repeats),
            rebinds: get(&self.rebinds),
            bind_failures: get(&self.bind_failures),
            suppressed_not_playing: get(&self.suppressed_not_playing),
            max_lag_ms: get(&self.max_lag_ms),
        }
    }
}
