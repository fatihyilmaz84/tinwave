//! Category and tier toggles ("collection respects toggles at the source").
//!
//! `docs/rules/privacy-and-signals.md`: a disabled signal is never constructed, not filtered
//! later. The bus checks [`SignalSettings::enabled`] before it calls the producer's constructor
//! closure, so a disabled category costs one atomic load and builds nothing.

use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use tinwave_core::Tier;

use crate::{SignalKind, SignalsError};

/// A Signal Center toggle: one per catalog section that holds signals.
///
/// The catalog groups its signals into these sections; section 7 (context) is not a signal.
///
/// The discriminant is the category's bit in [`SignalSettings::mask`], which is stored in
/// `signals.db` (`signal_settings.mask`): like a `SignalKind` discriminant it is **stable and
/// never reused**. A new category takes the next number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SignalCategory {
    /// Catalog section 1: volume, system volume, mute.
    Volume = 0,
    /// Catalog section 2: playback rate.
    Speed = 1,
    /// Catalog section 3: seek, skip, replay, pause, repeat.
    SeekSkipReplay = 2,
    /// Catalog section 4: queue, favorites, ratings, playlists, edits, search.
    QueueLibrary = 3,
    /// Catalog section 5: dwell, hover, impressions, views, window state.
    Browsing = 4,
    /// Catalog section 6: output device, EQ, exclusive mode, crossfade.
    OutputSound = 5,
    /// Catalog section 8: sessions, radio feedback, explanations.
    Session = 6,
}

impl SignalCategory {
    /// Every category, in catalog order.
    pub const ALL: [Self; 7] = [
        Self::Volume,
        Self::Speed,
        Self::SeekSkipReplay,
        Self::QueueLibrary,
        Self::Browsing,
        Self::OutputSound,
        Self::Session,
    ];

    /// The category a kind belongs to (its catalog section).
    ///
    /// Derived kinds belong to the section of their inputs. The match has no wildcard, so a new
    /// [`SignalKind`] must be placed here.
    #[must_use]
    pub const fn of(kind: SignalKind) -> Self {
        match kind {
            SignalKind::VolumeChanged
            | SignalKind::SystemVolumeChanged
            | SignalKind::MuteToggled
            | SignalKind::PerceivedLevel => Self::Volume,
            SignalKind::RateChanged | SignalKind::RateSession => Self::Speed,
            SignalKind::Skipped
            | SignalKind::Completed
            | SignalKind::SeekForward
            | SignalKind::SeekBack
            | SignalKind::Replayed
            | SignalKind::RepeatModeChanged
            | SignalKind::Paused
            | SignalKind::Resumed
            | SignalKind::PreviousPressed => Self::SeekSkipReplay,
            SignalKind::QueuedNext
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
            | SignalKind::SearchNoResult => Self::QueueLibrary,
            SignalKind::PageDwell
            | SignalKind::ArtworkHover
            | SignalKind::Impression
            | SignalKind::ViewOpened
            | SignalKind::WindowState => Self::Browsing,
            SignalKind::OutputDeviceChanged
            | SignalKind::EqChanged
            | SignalKind::ExclusiveModeToggled
            | SignalKind::CrossfadeChanged => Self::OutputSound,
            SignalKind::SessionStarted
            | SignalKind::SessionEnded
            | SignalKind::RadioFeedback
            | SignalKind::ExplanationOpened => Self::Session,
        }
    }

    const fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// Bit offset of the tier flags in the mask; categories use the bits below it.
const TIER_SHIFT: u32 = 8;

const fn tier_bit(tier: Tier) -> u32 {
    1 << (TIER_SHIFT + tier.number() as u32)
}

const fn all_categories() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < SignalCategory::ALL.len() {
        mask |= SignalCategory::ALL[i].bit();
        i += 1;
    }
    mask
}

/// Every bit a stored mask may have: the categories and the four tiers.
const KNOWN_BITS: u32 = all_categories()
    | tier_bit(Tier::T0)
    | tier_bit(Tier::T1)
    | tier_bit(Tier::T2)
    | tier_bit(Tier::T3);

/// Which categories and tiers are collected. Shared by every producer; flipping a toggle takes
/// effect on the next `enabled` call from any thread.
///
/// A kind is enabled when **both** its category and its tier are on. The default is the
/// privacy-rules default: every category on, tiers T0 and T1 on, T2 and T3 (opt-in) off.
#[derive(Debug)]
pub struct SignalSettings {
    /// Bits `0..7`: categories. Bits `8..12`: tiers T0..T3. One word so a check is one load.
    mask: AtomicU32,
}

impl SignalSettings {
    /// The defaults from `docs/rules/privacy-and-signals.md`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mask: AtomicU32::new(all_categories() | tier_bit(Tier::T0) | tier_bit(Tier::T1)),
        }
    }

    /// Settings restored from a stored [`mask`](Self::mask).
    ///
    /// # Errors
    /// [`SignalsError::InvalidValue`] if the mask has a bit that is neither a category nor a tier
    /// (a corrupt or newer-than-this-build value): the caller decides what to do, it is not
    /// silently trimmed.
    pub fn from_mask(mask: u32) -> Result<Self, SignalsError> {
        if mask & !KNOWN_BITS != 0 {
            return Err(SignalsError::InvalidValue {
                what: "SignalSettings mask",
                value: mask,
            });
        }
        Ok(Self {
            mask: AtomicU32::new(mask),
        })
    }

    /// The toggles as one word, for storage: bit `n` of `0..7` is the category with discriminant
    /// `n`, bit `8 + t` is tier `T<t>`. Defaults are `0x37F`. The layout is persisted
    /// (`signal_settings.mask`) and never changes; see [`SignalCategory`].
    #[must_use]
    pub fn mask(&self) -> u32 {
        self.mask.load(Ordering::Relaxed)
    }

    /// Everything off. The state of a user who disabled collection entirely.
    #[must_use]
    pub const fn all_off() -> Self {
        Self {
            mask: AtomicU32::new(0),
        }
    }

    /// Whether events of `kind` are collected right now.
    ///
    /// This is the check that runs before an event is constructed.
    #[must_use]
    pub fn enabled(&self, kind: SignalKind) -> bool {
        let need = SignalCategory::of(kind).bit() | tier_bit(kind.tier());
        self.mask.load(Ordering::Relaxed) & need == need
    }

    /// Whether a category toggle is on (independent of tiers).
    #[must_use]
    pub fn category_enabled(&self, category: SignalCategory) -> bool {
        self.mask.load(Ordering::Relaxed) & category.bit() != 0
    }

    /// Whether a tier toggle is on (independent of categories).
    #[must_use]
    pub fn tier_enabled(&self, tier: Tier) -> bool {
        self.mask.load(Ordering::Relaxed) & tier_bit(tier) != 0
    }

    /// Turns a category on or off.
    pub fn set_category(&self, category: SignalCategory, on: bool) {
        self.set_bits(category.bit(), on);
    }

    /// Turns a tier on or off.
    pub fn set_tier(&self, tier: Tier, on: bool) {
        self.set_bits(tier_bit(tier), on);
    }

    fn set_bits(&self, bits: u32, on: bool) {
        if on {
            self.mask.fetch_or(bits, Ordering::Relaxed);
        } else {
            self.mask.fetch_and(!bits, Ordering::Relaxed);
        }
    }
}

impl Default for SignalSettings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_mask_layout_is_frozen() {
        // These numbers are in users' databases (`signal_settings.mask`).
        let expected = [
            (SignalCategory::Volume, 0),
            (SignalCategory::Speed, 1),
            (SignalCategory::SeekSkipReplay, 2),
            (SignalCategory::QueueLibrary, 3),
            (SignalCategory::Browsing, 4),
            (SignalCategory::OutputSound, 5),
            (SignalCategory::Session, 6),
        ];
        for (category, bit) in expected {
            let s = SignalSettings::all_off();
            s.set_category(category, true);
            assert_eq!(s.mask(), 1 << bit, "{category:?}");
        }
        for tier in Tier::ALL {
            let s = SignalSettings::all_off();
            s.set_tier(tier, true);
            assert_eq!(s.mask(), 1 << (8 + u32::from(tier.number())), "{tier:?}");
        }
        assert_eq!(SignalSettings::new().mask(), 0x37F, "the defaults");
    }

    #[test]
    fn a_mask_round_trips_and_unknown_bits_are_refused() {
        let s = SignalSettings::new();
        s.set_category(SignalCategory::Browsing, false);
        s.set_tier(Tier::T2, true);
        let back = SignalSettings::from_mask(s.mask()).unwrap();
        assert_eq!(back.mask(), s.mask());
        for &kind in SignalKind::ALL {
            assert_eq!(back.enabled(kind), s.enabled(kind), "{kind}");
        }
        for bad in [1 << 7, 1 << 12, 1 << 31, u32::MAX] {
            assert_eq!(
                SignalSettings::from_mask(bad).unwrap_err(),
                SignalsError::InvalidValue {
                    what: "SignalSettings mask",
                    value: bad
                }
            );
        }
        assert_eq!(SignalSettings::from_mask(0).unwrap().mask(), 0);
        assert_eq!(SignalSettings::from_mask(0xF7F).unwrap().mask(), 0xF7F);
    }

    #[test]
    fn defaults_follow_the_privacy_rules() {
        let s = SignalSettings::new();
        for category in SignalCategory::ALL {
            assert!(s.category_enabled(category));
        }
        assert!(s.tier_enabled(Tier::T0) && s.tier_enabled(Tier::T1));
        assert!(!s.tier_enabled(Tier::T2) && !s.tier_enabled(Tier::T3));
        for &kind in SignalKind::ALL {
            assert!(s.enabled(kind), "{kind} is T0/T1 so it is on by default");
        }
    }

    #[test]
    fn a_category_toggle_disables_exactly_its_kinds() {
        for category in SignalCategory::ALL {
            let s = SignalSettings::new();
            s.set_category(category, false);
            for &kind in SignalKind::ALL {
                assert_eq!(
                    s.enabled(kind),
                    SignalCategory::of(kind) != category,
                    "{kind} with {category:?} off"
                );
            }
            s.set_category(category, true);
            assert!(SignalKind::ALL.iter().all(|&k| s.enabled(k)));
        }
    }

    #[test]
    fn a_tier_toggle_disables_exactly_its_kinds() {
        let s = SignalSettings::new();
        s.set_tier(Tier::T1, false);
        for &kind in SignalKind::ALL {
            assert_eq!(s.enabled(kind), kind.tier() != Tier::T1, "{kind}");
        }
        assert!(!s.enabled(SignalKind::OutputDeviceChanged));
        assert!(!s.enabled(SignalKind::PerceivedLevel), "derived T1");
        assert!(s.enabled(SignalKind::VolumeChanged));
    }

    #[test]
    fn opting_in_to_a_tier_does_not_enable_a_disabled_category() {
        let s = SignalSettings::new();
        s.set_category(SignalCategory::Browsing, false);
        s.set_tier(Tier::T2, true);
        assert!(!s.enabled(SignalKind::ViewOpened));
        assert!(s.enabled(SignalKind::Paused));
    }

    #[test]
    fn all_off_enables_nothing() {
        let s = SignalSettings::all_off();
        assert!(SignalKind::ALL.iter().all(|&k| !s.enabled(k)));
    }

    #[test]
    fn categories_match_the_catalog_sections() {
        use SignalCategory as C;
        let expected = [
            (SignalKind::VolumeChanged, C::Volume),
            (SignalKind::MuteToggled, C::Volume),
            (SignalKind::RateChanged, C::Speed),
            (SignalKind::SeekBack, C::SeekSkipReplay),
            (SignalKind::PreviousPressed, C::SeekSkipReplay),
            (SignalKind::SearchNoResult, C::QueueLibrary),
            (SignalKind::Impression, C::Browsing),
            (SignalKind::WindowState, C::Browsing),
            (SignalKind::CrossfadeChanged, C::OutputSound),
            (SignalKind::ExplanationOpened, C::Session),
        ];
        for (kind, category) in expected {
            assert_eq!(C::of(kind), category, "{kind}");
        }
    }

    #[test]
    fn settings_are_shared_across_threads() {
        let s = std::sync::Arc::new(SignalSettings::new());
        let t = {
            let s = s.clone();
            std::thread::spawn(move || s.set_category(SignalCategory::Volume, false))
        };
        t.join().unwrap();
        assert!(!s.enabled(SignalKind::VolumeChanged));
    }
}
