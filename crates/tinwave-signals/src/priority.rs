//! Which signals the bus may drop when it is under pressure.
//!
//! The bus has two bounded queues. [`Priority::Reserved`] events go to a queue that is only ever
//! used by them, so a flood of anything else cannot take their slots: that queue's capacity is
//! the "reserved capacity" of `docs/rules/rust.md` (Concurrency). [`Priority::Low`] events share
//! a second queue where, when it is full, the **oldest** low-priority event is dropped to make
//! room and the drop is counted per kind.
//!
//! # The rule
//!
//! An event is **reserved** when it belongs to the playback record or is an explicit user action
//! on the library: catalog sections 1 to 4 and 8 (volume, speed, seek/skip/replay, queue and
//! library behaviour, session and meta). Losing one corrupts play history or throws away
//! something the user did on purpose. Every reserved kind is tier T0.
//!
//! An event is **low** when it is ambient: section 5 events (`ViewOpened`, `WindowState`) and
//! section 6 (output device and sound settings). They are re-derivable from the next
//! `context_snapshot` or only change a setting, and they are rare, so they only get dropped when
//! the writer has stalled for a long time.
//!
//! Counter-class kinds (`Impression`, `ArtworkHover`, `PageDwell`) never enter a queue (they go
//! to `UiCounters`), and derived kinds are never emitted; both report `Low` only because the
//! match must be total.
//!
//! The `match` below has no wildcard arm: adding a [`SignalKind`] does not compile until its
//! priority is decided here.

use crate::SignalKind;

/// How the bus treats an event under pressure. See the module docs for the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Priority {
    /// Playback and explicit user actions: own queue, never evicted by other events.
    Reserved,
    /// Ambient events: the oldest is dropped (and counted) when the queue is full.
    Low,
}

impl Priority {
    /// The priority of a kind.
    #[must_use]
    pub const fn of(kind: SignalKind) -> Self {
        match kind {
            // 1. Volume
            SignalKind::VolumeChanged
            | SignalKind::SystemVolumeChanged
            | SignalKind::MuteToggled
            // 2. Speed
            | SignalKind::RateChanged
            // 3. Seek, skip, replay
            | SignalKind::Skipped
            | SignalKind::Completed
            | SignalKind::SeekForward
            | SignalKind::SeekBack
            | SignalKind::Replayed
            | SignalKind::RepeatModeChanged
            | SignalKind::Paused
            | SignalKind::Resumed
            | SignalKind::PreviousPressed
            // 4. Queue and library behaviour
            | SignalKind::QueuedNext
            | SignalKind::QueuedLater
            | SignalKind::RemovedFromQueue
            | SignalKind::QueueReordered
            | SignalKind::PlayStarted
            | SignalKind::Favorited
            | SignalKind::Unfavorited
            | SignalKind::Rated
            | SignalKind::PlaylistAdded
            | SignalKind::PlaylistRemoved
            | SignalKind::MetadataEdited
            | SignalKind::ArtworkChanged
            | SignalKind::SearchPerformed
            | SignalKind::SearchResultOpened
            | SignalKind::SearchNoResult
            // 8. Session and meta
            | SignalKind::SessionStarted
            | SignalKind::SessionEnded
            | SignalKind::RadioFeedback
            | SignalKind::ExplanationOpened => Self::Reserved,
            // 5. Browsing and attention (events only; the counters never queue)
            SignalKind::ViewOpened
            | SignalKind::WindowState
            | SignalKind::PageDwell
            | SignalKind::ArtworkHover
            | SignalKind::Impression
            // 6. Output and sound
            | SignalKind::OutputDeviceChanged
            | SignalKind::EqChanged
            | SignalKind::ExclusiveModeToggled
            | SignalKind::CrossfadeChanged
            // Derived: never emitted
            | SignalKind::PerceivedLevel
            | SignalKind::RateSession => Self::Low,
        }
    }
}

#[cfg(test)]
mod tests {
    use tinwave_core::Tier;

    use super::*;

    /// The complete list of low-priority event kinds. Anything not here is reserved.
    const LOW_EVENTS: &[SignalKind] = &[
        SignalKind::ViewOpened,
        SignalKind::WindowState,
        SignalKind::OutputDeviceChanged,
        SignalKind::EqChanged,
        SignalKind::ExclusiveModeToggled,
        SignalKind::CrossfadeChanged,
    ];

    #[test]
    fn low_priority_events_are_exactly_the_documented_list() {
        for kind in SignalKind::events() {
            let expected = if LOW_EVENTS.contains(&kind) {
                Priority::Low
            } else {
                Priority::Reserved
            };
            assert_eq!(Priority::of(kind), expected, "{kind}");
        }
    }

    #[test]
    fn every_reserved_kind_is_a_t0_event() {
        for kind in SignalKind::events() {
            if Priority::of(kind) == Priority::Reserved {
                assert_eq!(kind.tier(), Tier::T0, "{kind}");
            }
        }
    }

    #[test]
    fn the_playback_kinds_from_sections_1_to_3_and_session_are_reserved() {
        for kind in [
            SignalKind::VolumeChanged,
            SignalKind::SystemVolumeChanged,
            SignalKind::MuteToggled,
            SignalKind::RateChanged,
            SignalKind::Skipped,
            SignalKind::Completed,
            SignalKind::SeekForward,
            SignalKind::SeekBack,
            SignalKind::Replayed,
            SignalKind::Paused,
            SignalKind::Resumed,
            SignalKind::PlayStarted,
            SignalKind::SessionStarted,
            SignalKind::SessionEnded,
        ] {
            assert_eq!(Priority::of(kind), Priority::Reserved, "{kind}");
        }
    }
}
