//! Frozen name-to-value tables for everything that is persisted inside payloads.
//!
//! `SignalKind` discriminants have their own table in `kind.rs`. This module freezes the one-byte
//! enum codes, the `InputSource` tag bytes and the flag bit positions, each as explicit
//! `(name, value)` pairs. Comparing names with values (not just the list of values) is what
//! catches two variants swapping codes.
//!
//! Retiring: when a variant is deleted, move its `(name, value)` out of the frozen table and into
//! [`RETIRED`]. The tests then fail if any live variant is given that value again, so a persisted
//! number is never reused.

use std::collections::HashSet;

use crate::codec::{Pack, Writer};
use crate::enums::{
    ContextScope, EntityKind, InputSource, MetadataFields, OutputClass, OutputTransport,
    PlayOrigin, PlaylistKind, QueueOrigin, RadioFeedbackKind, RepeatMode, SearchScope, SectionKind,
    View, WindowStateKind,
};

/// Values of variants that were removed: `(enum or flag set, variant, value)`.
///
/// For coded enums the value is the code, for `InputSource` the tag byte, for flag sets the bit
/// position. Empty for now: nothing has been retired.
const RETIRED: &[(&str, &str, u8)] = &[];

/// Every persisted enum, flag set and tag set. `RETIRED` entries must refer to one of these.
const PERSISTED: &[&str] = &[
    "SectionKind",
    "QueueOrigin",
    "PlayOrigin",
    "RepeatMode",
    "OutputClass",
    "OutputTransport",
    "View",
    "PlaylistKind",
    "SearchScope",
    "EntityKind",
    "WindowStateKind",
    "RadioFeedbackKind",
    "InputSource",
    "MetadataFields",
    "ContextScope",
];

fn live<T: Copy>(
    all: &[T],
    name: impl Fn(T) -> &'static str,
    code: impl Fn(T) -> u8,
) -> Vec<(&'static str, u8)> {
    all.iter().map(|v| (name(*v), code(*v))).collect()
}

/// Checks a live name-to-value table against its frozen copy and against the retired values.
fn assert_frozen(what: &str, live: &[(&str, u8)], frozen: &[(&str, u8)]) {
    assert_eq!(
        live, frozen,
        "{what}: values are persisted in payloads and must never change"
    );
    let mut seen = HashSet::new();
    for (_, variant, value) in RETIRED.iter().filter(|(e, _, _)| *e == what) {
        assert!(
            seen.insert(*value),
            "{what}: retired value {value} ({variant}) is listed twice"
        );
        assert!(
            frozen.iter().all(|(_, v)| v != value),
            "{what}: value {value} was retired as {variant} and must not be reused"
        );
    }
}

#[test]
fn retired_entries_refer_to_known_enums() {
    for (what, variant, value) in RETIRED {
        assert!(
            PERSISTED.contains(what),
            "RETIRED entry {what}::{variant} = {value} names an unknown enum"
        );
    }
}

#[test]
fn enum_codes_are_frozen_by_name() {
    assert_frozen(
        "SectionKind",
        &live(SectionKind::ALL, SectionKind::name, SectionKind::code),
        &[
            ("Intro", 0),
            ("Verse", 1),
            ("Build", 2),
            ("Chorus", 3),
            ("Drop", 4),
            ("Bridge", 5),
            ("Breakdown", 6),
            ("Outro", 7),
            ("Unknown", 8),
        ],
    );
    assert_frozen(
        "QueueOrigin",
        &live(QueueOrigin::ALL, QueueOrigin::name, QueueOrigin::code),
        &[
            ("Manual", 0),
            ("Radio", 1),
            ("Album", 2),
            ("Playlist", 3),
            ("Shuffle", 4),
        ],
    );
    assert_frozen(
        "PlayOrigin",
        &live(PlayOrigin::ALL, PlayOrigin::name, PlayOrigin::code),
        &[
            ("Search", 0),
            ("Album", 1),
            ("Artist", 2),
            ("Playlist", 3),
            ("Radio", 4),
            ("Queue", 5),
            ("Resume", 6),
            ("Remote", 7),
        ],
    );
    assert_frozen(
        "RepeatMode",
        &live(RepeatMode::ALL, RepeatMode::name, RepeatMode::code),
        &[("Off", 0), ("All", 1), ("One", 2)],
    );
    assert_frozen(
        "OutputClass",
        &live(OutputClass::ALL, OutputClass::name, OutputClass::code),
        &[
            ("Headphones", 0),
            ("Speakers", 1),
            ("Bluetooth", 2),
            ("UsbDac", 3),
            ("Hdmi", 4),
            ("Builtin", 5),
            ("Virtual", 6),
            ("Unknown", 7),
        ],
    );
    assert_frozen(
        "OutputTransport",
        &live(
            OutputTransport::ALL,
            OutputTransport::name,
            OutputTransport::code,
        ),
        &[
            ("Bluetooth", 0),
            ("Usb", 1),
            ("Hda", 2),
            ("Virtual", 3),
            ("Other", 4),
        ],
    );
    assert_frozen(
        "View",
        &live(View::ALL, View::name, View::code),
        &[
            ("Home", 0),
            ("Library", 1),
            ("Album", 2),
            ("Artist", 3),
            ("Playlist", 4),
            ("Search", 5),
            ("Queue", 6),
            ("Radio", 7),
            ("NowPlaying", 8),
            ("Lyrics", 9),
            ("MiniPlayer", 10),
            ("Insights", 11),
        ],
    );
    assert_frozen(
        "PlaylistKind",
        &live(PlaylistKind::ALL, PlaylistKind::name, PlaylistKind::code),
        &[("Manual", 0), ("Smart", 1)],
    );
    assert_frozen(
        "SearchScope",
        &live(SearchScope::ALL, SearchScope::name, SearchScope::code),
        &[
            ("All", 0),
            ("Tracks", 1),
            ("Albums", 2),
            ("Artists", 3),
            ("Playlists", 4),
        ],
    );
    assert_frozen(
        "EntityKind",
        &live(EntityKind::ALL, EntityKind::name, EntityKind::code),
        &[("Track", 0), ("Album", 1), ("Artist", 2), ("Playlist", 3)],
    );
    assert_frozen(
        "WindowStateKind",
        &live(
            WindowStateKind::ALL,
            WindowStateKind::name,
            WindowStateKind::code,
        ),
        &[
            ("Focused", 0),
            ("Unfocused", 1),
            ("Minimized", 2),
            ("Tray", 3),
        ],
    );
    assert_frozen(
        "RadioFeedbackKind",
        &live(
            RadioFeedbackKind::ALL,
            RadioFeedbackKind::name,
            RadioFeedbackKind::code,
        ),
        &[
            ("NotNow", 0),
            ("NeverInContext", 1),
            ("MoreLikeThis", 2),
            ("ForgetThisPlay", 3),
        ],
    );
}

/// The first byte `InputSource` packs to, per variant.
fn input_tag(source: InputSource) -> u8 {
    let mut w = Writer::new();
    source.pack(&mut w).unwrap();
    w.finish().unwrap().as_bytes()[0]
}

#[test]
fn input_source_tags_are_frozen_by_name() {
    // Every variant, listed explicitly; the length check ties the list to `NAMES`.
    let all = [
        InputSource::Mouse,
        InputSource::Keyboard,
        InputSource::MediaKey,
        InputSource::Tray,
        InputSource::MiniPlayer,
        InputSource::Auto,
        InputSource::Remote(0),
        InputSource::System,
    ];
    assert_eq!(all.len(), InputSource::NAMES.len());
    let live: Vec<(&str, u8)> = all.iter().map(|s| (s.name(), input_tag(*s))).collect();
    assert_frozen(
        "InputSource",
        &live,
        &[
            ("Mouse", 0),
            ("Keyboard", 1),
            ("MediaKey", 2),
            ("Tray", 3),
            ("MiniPlayer", 4),
            ("Auto", 5),
            ("Remote", 6),
            ("System", 7),
        ],
    );
    // Remote is tag 6 plus the slot byte, whatever the slot.
    let mut w = Writer::new();
    InputSource::Remote(200).pack(&mut w).unwrap();
    assert_eq!(w.finish().unwrap().as_bytes(), [6, 200]);
}

/// Checks a flag set: names against bit positions, `ALL_BITS` against the list, retired bits.
fn assert_flags_frozen(what: &str, live: &[(&str, u16)], frozen: &[(&str, u8)], all_bits: u16) {
    let live_positions: Vec<(&str, u8)> = live
        .iter()
        .map(|(name, bits)| {
            assert_eq!(bits.count_ones(), 1, "{what}.{name} is not a single bit");
            (*name, u8::try_from(bits.trailing_zeros()).unwrap())
        })
        .collect();
    assert_frozen(what, &live_positions, frozen);
    let mask = frozen.iter().fold(0u16, |acc, (_, pos)| acc | (1 << pos));
    assert_eq!(
        mask, all_bits,
        "{what}: ALL_BITS and the flag list disagree"
    );
}

#[test]
fn flag_bit_positions_are_frozen_by_name() {
    assert_flags_frozen(
        "MetadataFields",
        &[
            ("TITLE", MetadataFields::TITLE.bits()),
            ("ARTIST", MetadataFields::ARTIST.bits()),
            ("ALBUM", MetadataFields::ALBUM.bits()),
            ("ALBUM_ARTIST", MetadataFields::ALBUM_ARTIST.bits()),
            ("GENRE", MetadataFields::GENRE.bits()),
            ("YEAR", MetadataFields::YEAR.bits()),
            ("TRACK_NUMBER", MetadataFields::TRACK_NUMBER.bits()),
            ("DISC_NUMBER", MetadataFields::DISC_NUMBER.bits()),
            ("COMPOSER", MetadataFields::COMPOSER.bits()),
            ("LYRICS", MetadataFields::LYRICS.bits()),
            ("COMMENT", MetadataFields::COMMENT.bits()),
        ],
        &[
            ("TITLE", 0),
            ("ARTIST", 1),
            ("ALBUM", 2),
            ("ALBUM_ARTIST", 3),
            ("GENRE", 4),
            ("YEAR", 5),
            ("TRACK_NUMBER", 6),
            ("DISC_NUMBER", 7),
            ("COMPOSER", 8),
            ("LYRICS", 9),
            ("COMMENT", 10),
        ],
        MetadataFields::ALL_BITS,
    );
    assert_flags_frozen(
        "ContextScope",
        &[
            ("DAYPART", ContextScope::DAYPART.bits()),
            ("WEEKDAY", ContextScope::WEEKDAY.bits()),
            ("SEASON", ContextScope::SEASON.bits()),
            ("WEATHER", ContextScope::WEATHER.bits()),
            ("DEVICE", ContextScope::DEVICE.bits()),
            ("ACTIVITY", ContextScope::ACTIVITY.bits()),
            ("PLACE", ContextScope::PLACE.bits()),
        ],
        &[
            ("DAYPART", 0),
            ("WEEKDAY", 1),
            ("SEASON", 2),
            ("WEATHER", 3),
            ("DEVICE", 4),
            ("ACTIVITY", 5),
            ("PLACE", 6),
        ],
        ContextScope::ALL_BITS,
    );
}
