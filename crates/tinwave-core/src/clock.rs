use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::UtcMillis;

/// A source of wall-clock time.
///
/// Domain code takes a `Clock` instead of reading the system time, so tests and replays are
/// deterministic. This is wall-clock time only; elapsed-time measurement (playback position)
/// belongs to the audio backend.
pub trait Clock: Send + Sync {
    /// The current time, UTC.
    fn now(&self) -> UtcMillis;
}

impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> UtcMillis {
        (**self).now()
    }
}

impl<C: Clock + ?Sized> Clock for &C {
    fn now(&self) -> UtcMillis {
        (**self).now()
    }
}

/// The real system clock. The only place in `tinwave-core` that reads the system time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UtcMillis {
        let ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
            // A system clock set before 1970 is reported as negative time, not hidden.
            Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |m| -m),
        };
        UtcMillis::new(ms)
    }
}

/// A clock that only moves when told to. For tests, replays and benchmarks.
///
/// Clones share the same time, so one handle can be given to the code under test and another kept
/// to advance it. Thread-safe.
#[derive(Debug, Clone)]
pub struct ManualClock {
    now: Arc<AtomicI64>,
}

impl ManualClock {
    /// A clock frozen at `start`.
    #[must_use]
    pub fn new(start: UtcMillis) -> Self {
        Self {
            now: Arc::new(AtomicI64::new(start.as_i64())),
        }
    }

    /// Jumps to `to` (may go backwards, to test clock-skew handling).
    pub fn set(&self, to: UtcMillis) {
        self.now.store(to.as_i64(), Ordering::SeqCst);
    }

    /// Moves forward by `ms` milliseconds (negative moves backwards). Saturates at the `i64` limits.
    pub fn advance(&self, ms: i64) {
        // The closure always returns `Some`, so the update cannot fail.
        let _ = self
            .now
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |cur| {
                Some(cur.saturating_add(ms))
            });
    }
}

impl Clock for ManualClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(self.now.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_is_frozen_until_moved() {
        let clock = ManualClock::new(UtcMillis::new(1_000));
        assert_eq!(clock.now(), UtcMillis::new(1_000));
        assert_eq!(clock.now(), UtcMillis::new(1_000));
        clock.advance(250);
        assert_eq!(clock.now(), UtcMillis::new(1_250));
        clock.set(UtcMillis::new(5));
        assert_eq!(clock.now(), UtcMillis::new(5));
        clock.advance(-10);
        assert_eq!(clock.now(), UtcMillis::new(-5));
    }

    #[test]
    fn manual_clock_advance_saturates() {
        let clock = ManualClock::new(UtcMillis::new(i64::MAX - 1));
        clock.advance(10);
        assert_eq!(clock.now(), UtcMillis::new(i64::MAX));
    }

    #[test]
    fn manual_clock_clones_share_time() {
        let a = ManualClock::new(UtcMillis::new(0));
        let b = a.clone();
        a.advance(7);
        assert_eq!(b.now(), UtcMillis::new(7));
        let shared: Arc<dyn Clock> = Arc::new(a.clone());
        a.advance(1);
        assert_eq!(shared.now(), UtcMillis::new(8));
    }

    #[test]
    fn manual_clock_is_thread_safe() {
        let clock = ManualClock::new(UtcMillis::new(0));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = clock.clone();
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        c.advance(1);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(clock.now(), UtcMillis::new(8_000));
    }

    #[test]
    fn system_clock_reports_a_plausible_recent_time() {
        // 2024-01-01T00:00:00Z; the machine clock is certainly later.
        assert!(SystemClock.now() > UtcMillis::new(1_704_067_200_000));
    }
}
