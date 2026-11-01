use std::fmt;

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// Privacy tier of a signal (see `docs/rules/privacy-and-signals.md`).
///
/// Ordered from least to most sensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// T0 Core: playback and library interactions. On by default.
    T0,
    /// T1 Local context: clock, power, idle, output device. On by default.
    T1,
    /// T2 Sensitive local: foreground-app category, network place, calendar. Opt-in.
    T2,
    /// T3 Network: weather. Opt-in; the only tier that touches the network.
    T3,
}

impl Tier {
    /// All tiers, least to most sensitive.
    pub const ALL: [Self; 4] = [Self::T0, Self::T1, Self::T2, Self::T3];

    /// The tier number, 0 to 3.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::T0 => 0,
            Self::T1 => 1,
            Self::T2 => 2,
            Self::T3 => 3,
        }
    }

    /// Whether the tier is off until the user opts in.
    #[must_use]
    pub const fn is_opt_in(self) -> bool {
        matches!(self, Self::T2 | Self::T3)
    }
}

impl TryFrom<u8> for Tier {
    type Error = CoreError;

    fn try_from(number: u8) -> Result<Self, Self::Error> {
        match number {
            0 => Ok(Self::T0),
            1 => Ok(Self::T1),
            2 => Ok(Self::T2),
            3 => Ok(Self::T3),
            other => Err(CoreError::InvalidTier(other)),
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}", self.number())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_round_trip() {
        for tier in Tier::ALL {
            assert_eq!(Tier::try_from(tier.number()).unwrap(), tier);
        }
        assert_eq!(Tier::try_from(4), Err(CoreError::InvalidTier(4)));
    }

    #[test]
    fn ordered_by_sensitivity() {
        assert!(Tier::T0 < Tier::T1 && Tier::T1 < Tier::T2 && Tier::T2 < Tier::T3);
    }

    #[test]
    fn opt_in_matches_privacy_rules() {
        assert!(!Tier::T0.is_opt_in() && !Tier::T1.is_opt_in());
        assert!(Tier::T2.is_opt_in() && Tier::T3.is_opt_in());
    }

    #[test]
    fn display_and_serde() {
        assert_eq!(Tier::T2.to_string(), "T2");
        let json = serde_json::to_string(&Tier::T3).unwrap();
        assert_eq!(json, "\"T3\"");
        assert_eq!(serde_json::from_str::<Tier>(&json).unwrap(), Tier::T3);
    }
}
