use std::fmt;

use serde::{Deserialize, Serialize};
use tinwave_core::Tier;

use crate::SignalsError;

/// How a signal is stored (see `docs/guides/signal-catalog.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StorageClass {
    /// **E**: a raw `signal_event` row (hot 90 days, then folded into play records and rollups).
    Event,
    /// **C**: aggregated in memory into daily `ui_counter` rows; never a raw event.
    Counter,
    /// **D**: computed from other data; not stored per occurrence.
    Derived,
}

/// `Some(tier)` for a declared tier; `None` for `Inputs`, meaning "derived from the inputs".
macro_rules! declared_tier {
    (Inputs) => {
        None
    };
    ($tier:ident) => {
        Some(Tier::$tier)
    };
}

macro_rules! signal_kinds {
    ($(
        $(#[$meta:meta])*
        $name:ident = $disc:literal, $class:ident, $tier:ident;
    )*) => {
        /// Every signal in the catalog that has a kind code.
        ///
        /// The discriminant is the value of `signal_event.kind` (and `ui_counter.kind`). It is
        /// **stable and never reused**: new kinds get new numbers, retired kinds keep theirs.
        /// Discriminants are grouped by catalog section, 16 per group, and stay below 128 so
        /// SQLite stores them in one byte. `0` is deliberately unused.
        ///
        /// `SignalDropped` is the one catalog signal without a kind: it is an in-memory
        /// diagnostics counter, never persisted, so it has no code.
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum SignalKind {
            $($(#[$meta])* $name = $disc,)*
        }

        impl SignalKind {
            /// All kinds, in discriminant order.
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];

            /// The catalog name of the kind.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self { $(Self::$name => stringify!($name),)* }
            }

            /// Storage class of the kind.
            #[must_use]
            pub const fn storage_class(self) -> StorageClass {
                match self { $(Self::$name => StorageClass::$class,)* }
            }

            /// The tier written in the table, `None` for derived kinds.
            const fn declared_tier(self) -> Option<Tier> {
                match self { $(Self::$name => declared_tier!($tier),)* }
            }
        }
    };
}

signal_kinds! {
    // 1. Volume
    /// In-app volume gesture ended.
    VolumeChanged = 1, Event, T0;
    /// OS master/endpoint volume changed while Tinwave plays.
    SystemVolumeChanged = 2, Event, T0;
    /// Mute switched on or off (app or system).
    MuteToggled = 3, Event, T0;
    /// Derived: effective loudness of a play.
    PerceivedLevel = 4, Derived, Inputs;

    // 2. Speed
    /// Playback-rate gesture ended.
    RateChanged = 17, Event, T0;
    /// Derived: time-weighted rate of a play.
    RateSession = 18, Derived, Inputs;

    // 3. Seek, skip, replay
    /// Next / track change before the end.
    Skipped = 33, Event, T0;
    /// Reached the end (or the last 3 %).
    Completed = 34, Event, T0;
    /// Seek of more than 2 s forward.
    SeekForward = 35, Event, T0;
    /// Seek of more than 2 s backward.
    SeekBack = 36, Event, T0;
    /// Previous / restart within 30 s of finishing, or a repeat-one loop.
    Replayed = 37, Event, T0;
    /// Repeat mode changed.
    RepeatModeChanged = 38, Event, T0;
    /// Playback paused.
    Paused = 39, Event, T0;
    /// Playback resumed.
    Resumed = 40, Event, T0;
    /// Previous track pressed.
    PreviousPressed = 41, Event, T0;

    // 4. Queue and library behaviour
    /// Play next.
    QueuedNext = 49, Event, T0;
    /// Add to queue.
    QueuedLater = 50, Event, T0;
    /// Removed from the queue before it played.
    RemovedFromQueue = 51, Event, T0;
    /// Dragged to a new queue position.
    QueueReordered = 52, Event, T0;
    /// Any play start.
    PlayStarted = 53, Event, T0;
    /// Heart on.
    Favorited = 54, Event, T0;
    /// Heart off.
    Unfavorited = 55, Event, T0;
    /// Rating set.
    Rated = 56, Event, T0;
    /// Added to a playlist.
    PlaylistAdded = 57, Event, T0;
    /// Removed from a playlist.
    PlaylistRemoved = 58, Event, T0;
    /// Tags edited.
    MetadataEdited = 59, Event, T0;
    /// Artwork edited.
    ArtworkChanged = 60, Event, T0;
    /// Search submitted or settled.
    SearchPerformed = 61, Event, T0;
    /// Search result clicked.
    SearchResultOpened = 62, Event, T0;
    /// Search returned nothing.
    SearchNoResult = 63, Event, T0;

    // 5. Browsing and attention
    /// Counter: left an album/artist/playlist page.
    PageDwell = 65, Counter, T0;
    /// Counter: artwork hover of 800 ms or more.
    ArtworkHover = 66, Counter, T0;
    /// Counter: item visible for 500 ms or more in a settled list.
    Impression = 67, Counter, T0;
    /// Lyrics, Now Playing, mini-player or Insights opened.
    ViewOpened = 68, Event, T0;
    /// App focused / unfocused / minimized / in tray.
    WindowState = 69, Event, T1;

    // 6. Output and sound
    /// Default or chosen output device changed.
    OutputDeviceChanged = 81, Event, T1;
    /// EQ preset or band changed.
    EqChanged = 82, Event, T0;
    /// Exclusive / bit-perfect mode toggled.
    ExclusiveModeToggled = 83, Event, T0;
    /// Crossfade / gapless setting changed.
    CrossfadeChanged = 84, Event, T0;

    // 8. Session and meta (7 is context fields, which are not signals)
    /// First play after 30 minutes of silence.
    SessionStarted = 113, Event, T0;
    /// 30 minutes of silence.
    SessionEnded = 114, Event, T0;
    /// Direct radio correction ("Not now", "More like this", ...).
    RadioFeedback = 115, Event, T0;
    /// "Why this?" opened.
    ExplanationOpened = 116, Event, T0;
}

impl SignalKind {
    /// Privacy tier of the kind. Tier is derived from the kind and never stored.
    ///
    /// Rule: a derived kind takes the **highest tier among its inputs**
    /// ([`derived_inputs`](Self::derived_inputs)), because it can reveal everything its inputs
    /// reveal. Every other kind has the tier written in the table above.
    #[must_use]
    pub const fn tier(self) -> Tier {
        match self.declared_tier() {
            Some(tier) => tier,
            None => {
                let inputs = self.derived_inputs();
                let mut highest = Tier::T0;
                let mut i = 0;
                while i < inputs.len() {
                    let tier = inputs[i].tier();
                    if tier.number() > highest.number() {
                        highest = tier;
                    }
                    i += 1;
                }
                highest
            }
        }
    }

    /// The raw-event kinds a derived kind is computed from; empty for events and counters.
    ///
    /// These drive [`tier`](Self::tier). Inputs must be raw events (never other derived kinds).
    /// - `PerceivedLevel` = volume, system volume and the output device (its endpoint gain is
    ///   device context, T1).
    /// - `RateSession` = rate changes.
    #[must_use]
    pub const fn derived_inputs(self) -> &'static [SignalKind] {
        match self {
            Self::PerceivedLevel => &[
                Self::VolumeChanged,
                Self::SystemVolumeChanged,
                Self::OutputDeviceChanged,
            ],
            Self::RateSession => &[Self::RateChanged],
            _ => &[],
        }
    }

    /// The stable value stored in `signal_event.kind`.
    #[must_use]
    pub const fn discriminant(self) -> u8 {
        self as u8
    }

    /// The kind for a stored discriminant.
    ///
    /// # Errors
    /// [`SignalsError::UnknownKind`] if no kind has this number.
    pub fn from_discriminant(value: u8) -> Result<Self, SignalsError> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.discriminant() == value)
            .ok_or(SignalsError::UnknownKind(value))
    }

    /// Whether the kind is stored as a raw `signal_event` row (storage class **E**).
    #[must_use]
    pub const fn is_event(self) -> bool {
        matches!(self.storage_class(), StorageClass::Event)
    }

    /// Iterator over the kinds stored as raw events.
    pub fn events() -> impl Iterator<Item = Self> {
        Self::ALL.iter().copied().filter(|kind| kind.is_event())
    }
}

impl TryFrom<u8> for SignalKind {
    type Error = SignalsError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::from_discriminant(value)
    }
}

impl From<SignalKind> for u8 {
    fn from(kind: SignalKind) -> Self {
        kind.discriminant()
    }
}

impl fmt::Display for SignalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frozen `(name, discriminant)` table. A change to any value here is a breaking change to
    /// stored data and must never happen: add a new kind with a new number instead.
    const FROZEN: &[(&str, u8)] = &[
        ("VolumeChanged", 1),
        ("SystemVolumeChanged", 2),
        ("MuteToggled", 3),
        ("PerceivedLevel", 4),
        ("RateChanged", 17),
        ("RateSession", 18),
        ("Skipped", 33),
        ("Completed", 34),
        ("SeekForward", 35),
        ("SeekBack", 36),
        ("Replayed", 37),
        ("RepeatModeChanged", 38),
        ("Paused", 39),
        ("Resumed", 40),
        ("PreviousPressed", 41),
        ("QueuedNext", 49),
        ("QueuedLater", 50),
        ("RemovedFromQueue", 51),
        ("QueueReordered", 52),
        ("PlayStarted", 53),
        ("Favorited", 54),
        ("Unfavorited", 55),
        ("Rated", 56),
        ("PlaylistAdded", 57),
        ("PlaylistRemoved", 58),
        ("MetadataEdited", 59),
        ("ArtworkChanged", 60),
        ("SearchPerformed", 61),
        ("SearchResultOpened", 62),
        ("SearchNoResult", 63),
        ("PageDwell", 65),
        ("ArtworkHover", 66),
        ("Impression", 67),
        ("ViewOpened", 68),
        ("WindowState", 69),
        ("OutputDeviceChanged", 81),
        ("EqChanged", 82),
        ("ExclusiveModeToggled", 83),
        ("CrossfadeChanged", 84),
        ("SessionStarted", 113),
        ("SessionEnded", 114),
        ("RadioFeedback", 115),
        ("ExplanationOpened", 116),
    ];

    #[test]
    fn discriminants_are_frozen() {
        let actual: Vec<(&str, u8)> = SignalKind::ALL
            .iter()
            .map(|kind| (kind.name(), kind.discriminant()))
            .collect();
        assert_eq!(
            actual, FROZEN,
            "SignalKind discriminants are persisted and must never change"
        );
    }

    #[test]
    fn discriminants_are_unique_nonzero_and_fit_one_sqlite_byte() {
        let mut seen = std::collections::HashSet::new();
        for kind in SignalKind::ALL {
            let d = kind.discriminant();
            assert!(d != 0 && d < 128, "{kind} = {d}");
            assert!(seen.insert(d), "{kind} reuses {d}");
        }
    }

    #[test]
    fn discriminant_round_trips() {
        for &kind in SignalKind::ALL {
            assert_eq!(SignalKind::from_discriminant(kind.discriminant()), Ok(kind));
        }
        assert_eq!(
            SignalKind::from_discriminant(0),
            Err(SignalsError::UnknownKind(0))
        );
        assert_eq!(
            SignalKind::try_from(200),
            Err(SignalsError::UnknownKind(200))
        );
    }

    #[test]
    fn tiers_follow_the_catalog_privacy_rules() {
        assert_eq!(SignalKind::WindowState.tier(), Tier::T1);
        assert_eq!(SignalKind::OutputDeviceChanged.tier(), Tier::T1);
        assert_eq!(SignalKind::VolumeChanged.tier(), Tier::T0);
        assert!(SignalKind::ALL.iter().all(|kind| kind.tier() <= Tier::T1));
    }

    #[test]
    fn derived_kinds_take_the_highest_tier_of_their_inputs() {
        assert_eq!(SignalKind::PerceivedLevel.tier(), Tier::T1);
        assert_eq!(SignalKind::RateSession.tier(), Tier::T0);
        for &kind in SignalKind::ALL {
            let inputs = kind.derived_inputs();
            if kind.storage_class() == StorageClass::Derived {
                assert!(!inputs.is_empty(), "{kind} has no inputs");
                assert!(
                    inputs.iter().all(|i| i.is_event()),
                    "{kind}: inputs are raw events"
                );
                let highest = inputs.iter().map(|i| i.tier()).max().unwrap();
                assert_eq!(kind.tier(), highest, "{kind}");
            } else {
                assert!(inputs.is_empty(), "{kind} is not derived");
            }
        }
    }

    #[test]
    fn storage_classes() {
        for counter in [
            SignalKind::PageDwell,
            SignalKind::ArtworkHover,
            SignalKind::Impression,
        ] {
            assert_eq!(counter.storage_class(), StorageClass::Counter);
            assert!(!counter.is_event());
        }
        for derived in [SignalKind::PerceivedLevel, SignalKind::RateSession] {
            assert_eq!(derived.storage_class(), StorageClass::Derived);
        }
        assert_eq!(SignalKind::events().count(), 38);
    }

    #[test]
    fn display_and_serde_use_the_catalog_name() {
        assert_eq!(SignalKind::SeekBack.to_string(), "SeekBack");
        let json = serde_json::to_string(&SignalKind::SeekBack).unwrap();
        assert_eq!(json, "\"SeekBack\"");
        assert_eq!(
            serde_json::from_str::<SignalKind>(&json).unwrap(),
            SignalKind::SeekBack
        );
    }
}
