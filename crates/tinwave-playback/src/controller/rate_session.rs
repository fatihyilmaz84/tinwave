//! `RateSession`: the time-weighted playback rate of one heard play (signal catalog section 2).
//!
//! It is a *derived* signal: the bus refuses it as an event (`SignalsError::NotAnEvent`), so the
//! controller hands each one over through [`RateSessionQueue`] instead, and whoever folds plays into
//! records (or the taste engine) takes them with [`Controller::take_rate_sessions`](super::Controller::take_rate_sessions)
//! or [`ControllerHandle::take_rate_sessions`](super::ControllerHandle::take_rate_sessions).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::types::RateSession;

/// Rate sessions kept for the consumer. A consumer that does not collect them loses the oldest
/// (counted), so an idle one costs a bounded 64 entries and never blocks playback.
pub(super) const MAX_PENDING_RATE_SESSIONS: usize = 64;

#[derive(Debug, Default)]
struct Inner {
    items: VecDeque<RateSession>,
    dropped: u64,
}

/// The hand-over point between the controller thread (producer) and the consumer. Cloning shares
/// the queue. Locks are held for a push or a swap of the buffer, never across anything slow.
#[derive(Debug, Clone, Default)]
pub(super) struct RateSessionQueue {
    inner: Arc<Mutex<Inner>>,
}

impl RateSessionQueue {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn push(&self, session: RateSession) {
        let mut inner = self.lock();
        if inner.items.len() >= MAX_PENDING_RATE_SESSIONS {
            inner.items.pop_front();
            inner.dropped = inner.dropped.saturating_add(1);
        }
        inner.items.push_back(session);
    }

    /// Everything waiting, oldest first.
    pub fn take(&self) -> Vec<RateSession> {
        std::mem::take(&mut self.lock().items).into()
    }

    /// Sessions dropped because more than [`MAX_PENDING_RATE_SESSIONS`] were waiting.
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }
}
