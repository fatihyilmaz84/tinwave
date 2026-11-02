//! Listening-session bookkeeping (catalog section 8): the pure part, no bus and no clock.
//!
//! A session starts with the first play after 30 minutes of silence (or the first ever) and ends
//! after 30 minutes of silence. *Silence* is the absence of audible playback: a pause, a stop and
//! the end of the queue all start it; a track change that follows at once does not matter, because
//! the next `TrackStarted` clears it again. A session therefore survives a pause shorter than 30
//! minutes, and a pause of exactly 30 minutes ends it.

use tinwave_core::{SessionId, UtcMillis};

/// Silence that ends a session, in milliseconds.
pub(super) const SESSION_SILENCE_MS: i64 = 30 * 60 * 1000;

/// A listening session in progress.
#[derive(Debug, Clone, Copy)]
pub(super) struct Session {
    pub id: SessionId,
    /// The time embedded in `id`: no event of the session is stamped before it, so stored
    /// `ts_off` values are never negative.
    pub started_at: UtcMillis,
    /// `TrackStarted` events since the session began.
    pub tracks: u32,
    /// Since when nothing has been audible; `None` while something is.
    pub silence_since: Option<UtcMillis>,
}

impl Session {
    pub fn new(id: SessionId) -> Self {
        Self {
            id,
            started_at: id.created_at(),
            tracks: 0,
            silence_since: None,
        }
    }

    /// When the session ended, if the silence has lasted 30 minutes by `now`.
    pub fn ended_by(&self, now: UtcMillis) -> Option<UtcMillis> {
        let since = self.silence_since?;
        let end = since.as_i64().saturating_add(SESSION_SILENCE_MS);
        // A clock that moved backwards cannot measure silence; it is not a reason to end.
        (now.as_i64() >= end).then(|| UtcMillis::new(end))
    }

    /// An event time that is never before the session started.
    pub fn stamp(&self, at: UtcMillis) -> UtcMillis {
        at.max(self.started_at)
    }

    /// Whole minutes of listening from the start to `until` (the last audible moment).
    pub fn minutes(&self, until: UtcMillis) -> u16 {
        let ms = until
            .as_i64()
            .saturating_sub(self.started_at.as_i64())
            .max(0);
        u16::try_from(ms / 60_000).unwrap_or(u16::MAX)
    }

    /// Records whether something is audible at `at`.
    pub fn set_audible(&mut self, at: UtcMillis, audible: bool) {
        if audible {
            self.silence_since = None;
        } else if self.silence_since.is_none() {
            self.silence_since = Some(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let at = UtcMillis::new(1_000_000);
        Session::new(SessionId::from_parts(at, [7; 10]).unwrap())
    }

    #[test]
    fn silence_ends_the_session_at_exactly_thirty_minutes() {
        let mut s = session();
        let t = UtcMillis::new(2_000_000);
        s.set_audible(t, false);
        let just_before = UtcMillis::new(t.as_i64() + SESSION_SILENCE_MS - 1);
        let exactly = UtcMillis::new(t.as_i64() + SESSION_SILENCE_MS);
        assert_eq!(s.ended_by(just_before), None);
        assert_eq!(s.ended_by(exactly), Some(exactly));
    }

    #[test]
    fn audible_playback_clears_the_silence() {
        let mut s = session();
        s.set_audible(UtcMillis::new(2_000_000), false);
        s.set_audible(UtcMillis::new(2_001_000), true);
        assert_eq!(s.silence_since, None);
        assert_eq!(s.ended_by(UtcMillis::new(i64::MAX)), None);
    }

    #[test]
    fn silence_is_measured_from_its_first_moment() {
        let mut s = session();
        s.set_audible(UtcMillis::new(2_000_000), false);
        s.set_audible(UtcMillis::new(2_500_000), false);
        assert_eq!(s.silence_since, Some(UtcMillis::new(2_000_000)));
    }

    #[test]
    fn minutes_are_whole_and_saturate() {
        let s = session();
        assert_eq!(s.minutes(UtcMillis::new(1_000_000 + 59_999)), 0);
        assert_eq!(s.minutes(UtcMillis::new(1_000_000 + 60_000)), 1);
        assert_eq!(s.minutes(UtcMillis::new(0)), 0, "before the start is 0");
        assert_eq!(s.minutes(UtcMillis::new(i64::MAX)), u16::MAX);
    }

    #[test]
    fn stamps_never_precede_the_session() {
        let s = session();
        assert_eq!(s.stamp(UtcMillis::new(5)), s.started_at);
        assert_eq!(
            s.stamp(UtcMillis::new(2_000_000)),
            UtcMillis::new(2_000_000)
        );
    }
}
