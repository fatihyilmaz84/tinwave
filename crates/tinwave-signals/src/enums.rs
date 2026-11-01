//! Small closed enums and flag sets used in payloads.
//!
//! Every enum has explicit one-byte codes. Codes are persisted inside payloads: like
//! [`SignalKind`](crate::SignalKind) discriminants they are stable and never reused. A new
//! variant takes a new code; changing the meaning of an existing code needs a new payload
//! version (see `docs/guides/adding-a-signal.md`).

use serde::{Deserialize, Serialize};

use crate::codec::{Pack, Reader, Writer};
use crate::SignalsError;

/// Defines an enum with explicit one-byte codes, its lookup helpers and its [`Pack`] impl.
macro_rules! coded_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $($(#[$vmeta:meta])* $variant:ident = $code:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
        }

        impl $name {
            /// All variants, in code order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The stable one-byte code.
            #[must_use]
            pub const fn code(self) -> u8 {
                match self { $(Self::$variant => $code,)* }
            }

            /// The variant name as written in the signal catalog.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => stringify!($variant),)* }
            }

            /// The variant for a stored code.
            ///
            /// # Errors
            /// [`SignalsError::InvalidValue`] if no variant has this code.
            pub fn from_code(code: u8) -> Result<Self, SignalsError> {
                match code {
                    $($code => Ok(Self::$variant),)*
                    other => Err(SignalsError::InvalidValue {
                        what: stringify!($name),
                        value: u32::from(other),
                    }),
                }
            }
        }

        impl Pack for $name {
            fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
                w.byte(self.code())
            }
            fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
                Self::from_code(r.byte()?)
            }
        }
    };
}

/// Defines a `u16` flag set. Unknown bits are rejected on decode.
macro_rules! flags_u16 {
    (
        $(#[$meta:meta])*
        $name:ident {
            $($(#[$cmeta:meta])* $flag:ident = $bit:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "u16", into = "u16")]
        pub struct $name(u16);

        impl $name {
            $($(#[$cmeta])* pub const $flag: Self = Self(1 << $bit);)*

            /// No flags set.
            pub const EMPTY: Self = Self(0);
            /// Every defined flag.
            pub const ALL_BITS: u16 = 0 $(| (1 << $bit))*;

            /// Validates a raw bit set.
            ///
            /// # Errors
            /// [`SignalsError::InvalidValue`] if a bit outside the defined flags is set.
            pub const fn from_bits(bits: u16) -> Result<Self, SignalsError> {
                if bits & !Self::ALL_BITS != 0 {
                    Err(SignalsError::InvalidValue {
                        what: stringify!($name),
                        value: bits as u32,
                    })
                } else {
                    Ok(Self(bits))
                }
            }

            /// The raw bit set.
            #[must_use]
            pub const fn bits(self) -> u16 {
                self.0
            }

            /// Whether every flag in `other` is set.
            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// This set plus every flag in `other`.
            #[must_use]
            pub const fn with(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            /// Whether no flag is set.
            #[must_use]
            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }
        }

        impl TryFrom<u16> for $name {
            type Error = SignalsError;
            fn try_from(bits: u16) -> Result<Self, Self::Error> {
                Self::from_bits(bits)
            }
        }

        impl From<$name> for u16 {
            fn from(flags: $name) -> Self {
                flags.0
            }
        }
    };
}

coded_enum! {
    /// Section label from audio analysis (`docs/guides/audio-analysis.md`).
    SectionKind {
        Intro = 0,
        Verse = 1,
        Build = 2,
        Chorus = 3,
        Drop = 4,
        Bridge = 5,
        Breakdown = 6,
        Outro = 7,
        Unknown = 8,
    }
}

/// A section of a track: its label plus its index among the track's sections.
///
/// Packed into one byte (label in the low nibble, index in the high nibble), so the index is
/// `0..=15`, matching the "sections <= 16" cap on audio features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "SectionWire")]
pub struct Section {
    kind: SectionKind,
    index: u8,
}

#[derive(Deserialize)]
struct SectionWire {
    kind: SectionKind,
    index: u8,
}

impl Section {
    /// Largest section index.
    pub const MAX_INDEX: u8 = 15;
    /// A section that is not known (no analysis yet).
    pub const UNKNOWN: Self = Self {
        kind: SectionKind::Unknown,
        index: 0,
    };

    /// Builds a section.
    ///
    /// # Errors
    /// [`SignalsError::InvalidValue`] if `index` is above 15.
    pub const fn new(kind: SectionKind, index: u8) -> Result<Self, SignalsError> {
        if index > Self::MAX_INDEX {
            Err(SignalsError::InvalidValue {
                what: "Section index",
                value: index as u32,
            })
        } else {
            Ok(Self { kind, index })
        }
    }

    /// The section label.
    #[must_use]
    pub const fn kind(self) -> SectionKind {
        self.kind
    }

    /// The section index within the track.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.index
    }
}

impl TryFrom<SectionWire> for Section {
    type Error = SignalsError;

    fn try_from(wire: SectionWire) -> Result<Self, Self::Error> {
        Self::new(wire.kind, wire.index)
    }
}

/// What caused an interaction (catalog "Common enums").
///
/// `Remote` carries the slot (`0..=255`) of the controlling device in the local paired-device
/// table, not a UUID: a UUID does not fit the 12-byte payload budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputSource {
    /// Pointer in the main window.
    Mouse,
    /// Keyboard shortcut.
    Keyboard,
    /// Hardware media key.
    MediaKey,
    /// System tray menu.
    Tray,
    /// Mini-player.
    MiniPlayer,
    /// Another Tinwave device acting as a remote; the slot of that device.
    Remote(u8),
    /// The operating system (volume keys, flyout and mixer are indistinguishable on Windows,
    /// ADR 0004). Used by `SystemVolumeChanged` and system mute.
    System,
    /// Tinwave itself (auto-advance, radio, sleep timer).
    Auto,
}

impl InputSource {
    /// The variant names as written in the catalog (`Remote` without its argument).
    pub const NAMES: &'static [&'static str] = &[
        "Mouse",
        "Keyboard",
        "MediaKey",
        "Tray",
        "MiniPlayer",
        "Remote",
        "System",
        "Auto",
    ];

    /// The variant name as written in the catalog.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mouse => "Mouse",
            Self::Keyboard => "Keyboard",
            Self::MediaKey => "MediaKey",
            Self::Tray => "Tray",
            Self::MiniPlayer => "MiniPlayer",
            Self::Remote(_) => "Remote",
            Self::System => "System",
            Self::Auto => "Auto",
        }
    }
}

coded_enum! {
    /// Where the queue entry that was skipped came from.
    QueueOrigin {
        Manual = 0,
        Radio = 1,
        Album = 2,
        Playlist = 3,
        Shuffle = 4,
    }
}

coded_enum! {
    /// Why a play started (also used for a session's `start_origin`).
    PlayOrigin {
        Search = 0,
        Album = 1,
        Artist = 2,
        Playlist = 3,
        Radio = 4,
        Queue = 5,
        Resume = 6,
        Remote = 7,
    }
}

coded_enum! {
    /// Repeat mode.
    RepeatMode {
        Off = 0,
        All = 1,
        One = 2,
    }
}

coded_enum! {
    /// Class of an audio output endpoint (`OutputDeviceChanged.class`,
    /// `SystemVolumeChanged.endpoint_class`). `UsbDac` means an external DAC, USB or digital
    /// out (ADR 0004).
    OutputClass {
        Headphones = 0,
        Speakers = 1,
        Bluetooth = 2,
        UsbDac = 3,
        Hdmi = 4,
        Builtin = 5,
        Virtual = 6,
        Unknown = 7,
    }
}

coded_enum! {
    /// How an output endpoint is attached (`OutputDeviceChanged.transport`, ADR 0004).
    OutputTransport {
        Bluetooth = 0,
        Usb = 1,
        Hda = 2,
        Virtual = 3,
        Other = 4,
    }
}

coded_enum! {
    /// A top-level view or surface (`from_view`, `ViewOpened.view`).
    View {
        Home = 0,
        Library = 1,
        Album = 2,
        Artist = 3,
        Playlist = 4,
        Search = 5,
        Queue = 6,
        Radio = 7,
        NowPlaying = 8,
        Lyrics = 9,
        MiniPlayer = 10,
        Insights = 11,
    }
}

coded_enum! {
    /// Kind of playlist an edit touched.
    PlaylistKind {
        Manual = 0,
        Smart = 1,
    }
}

coded_enum! {
    /// Scope a search was run in.
    SearchScope {
        All = 0,
        Tracks = 1,
        Albums = 2,
        Artists = 3,
        Playlists = 4,
    }
}

coded_enum! {
    /// Kind of entity a search result points at.
    EntityKind {
        Track = 0,
        Album = 1,
        Artist = 2,
        Playlist = 3,
    }
}

coded_enum! {
    /// Window state transitions (`WindowState.state`).
    WindowStateKind {
        Focused = 0,
        Unfocused = 1,
        Minimized = 2,
        Tray = 3,
    }
}

coded_enum! {
    /// The correction the listener gave the radio.
    RadioFeedbackKind {
        NotNow = 0,
        NeverInContext = 1,
        MoreLikeThis = 2,
        ForgetThisPlay = 3,
    }
}

flags_u16! {
    /// Which tag fields a `MetadataEdited` event touched (names only, never values).
    MetadataFields {
        /// Track title.
        TITLE = 0,
        /// Track artist.
        ARTIST = 1,
        /// Album title.
        ALBUM = 2,
        /// Album artist.
        ALBUM_ARTIST = 3,
        /// Genre.
        GENRE = 4,
        /// Year or date.
        YEAR = 5,
        /// Track number.
        TRACK_NUMBER = 6,
        /// Disc number.
        DISC_NUMBER = 7,
        /// Composer.
        COMPOSER = 8,
        /// Lyrics.
        LYRICS = 9,
        /// Comment.
        COMMENT = 10,
    }
}

flags_u16! {
    /// Which context dimensions a `RadioFeedback` applies to. Empty means "this play only".
    ContextScope {
        /// Time-of-day bucket.
        DAYPART = 0,
        /// Weekday versus weekend.
        WEEKDAY = 1,
        /// Season.
        SEASON = 2,
        /// Weather bucket.
        WEATHER = 3,
        /// Output device class.
        DEVICE = 4,
        /// Activity (foreground app category, fullscreen, focus mode).
        ACTIVITY = 5,
        /// Network place.
        PLACE = 6,
    }
}

impl Pack for MetadataFields {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        self.bits().pack(w)
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        Self::from_bits(u16::unpack(r)?)
    }
}

impl Pack for ContextScope {
    fn pack(&self, w: &mut Writer) -> Result<(), SignalsError> {
        self.bits().pack(w)
    }
    fn unpack(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
        Self::from_bits(u16::unpack(r)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_unknown_codes_fail() {
        for &v in View::ALL {
            assert_eq!(View::from_code(v.code()), Ok(v));
        }
        assert_eq!(
            View::from_code(12),
            Err(SignalsError::InvalidValue {
                what: "View",
                value: 12
            })
        );
        assert_eq!(PlayOrigin::Radio.name(), "Radio");
    }

    #[test]
    fn section_validates_index() {
        assert!(Section::new(SectionKind::Drop, 15).is_ok());
        assert!(Section::new(SectionKind::Drop, 16).is_err());
        assert_eq!(Section::UNKNOWN.kind(), SectionKind::Unknown);
        let json = serde_json::to_string(&Section::new(SectionKind::Verse, 2).unwrap()).unwrap();
        assert_eq!(json, r#"{"kind":"Verse","index":2}"#);
        assert!(serde_json::from_str::<Section>(r#"{"kind":"Verse","index":16}"#).is_err());
    }

    #[test]
    fn flags_reject_unknown_bits() {
        let f = MetadataFields::TITLE.with(MetadataFields::GENRE);
        assert!(f.contains(MetadataFields::GENRE));
        assert!(!f.contains(MetadataFields::YEAR));
        assert_eq!(f.bits(), 0b1_0001);
        assert!(MetadataFields::from_bits(1 << 11).is_err());
        assert!(ContextScope::from_bits(1 << 7).is_err());
        assert!(ContextScope::EMPTY.is_empty());
        assert!(serde_json::from_str::<ContextScope>("128").is_err());
    }

    #[test]
    fn input_source_names_cover_every_variant() {
        let all = [
            InputSource::Mouse,
            InputSource::Keyboard,
            InputSource::MediaKey,
            InputSource::Tray,
            InputSource::MiniPlayer,
            InputSource::Remote(0),
            InputSource::System,
            InputSource::Auto,
        ];
        let names: Vec<&str> = all.iter().map(|v| v.name()).collect();
        assert_eq!(names, InputSource::NAMES);
    }
}
