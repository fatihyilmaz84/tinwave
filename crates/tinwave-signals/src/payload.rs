//! Payload structs per signal kind and the versioned codec entry points.
//!
//! The byte layout of every struct is its field list, in order (see [`crate::codec`] for the
//! field encodings). Version 1 byte costs are noted in each struct's doc; the worst case is
//! asserted in tests to stay within [`MAX_PAYLOAD_LEN`](crate::MAX_PAYLOAD_LEN).

use serde::{Deserialize, Serialize};

use crate::codec::{EncodedPayload, Pack, Reader, Writer};
use crate::enums::{
    ContextScope, EntityKind, InputSource, MetadataFields, OutputClass, OutputTransport,
    PlayOrigin, PlaylistKind, QueueOrigin, RadioFeedbackKind, RepeatMode, SearchScope, Section,
    View, WindowStateKind,
};
use crate::quant::{ElapsedDs, Fraction, GestureTime, Hash32, PositionDs, Rate, Stars, Volume};
use crate::{SignalKind, SignalsError};

/// One field of a payload: its name in code and the catalog field it stores.
///
/// `catalog` is the name used in the Payload column of `docs/guides/signal-catalog.md`. It
/// differs from `name` where the stored form is quantized (`ms` -> `ds`) or reduced to a hash.
/// A test parses the catalog and checks these names, so the catalog and the code cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    /// Field name in the Rust struct and in JSON export.
    pub name: &'static str,
    /// Field name in the signal catalog this field stores.
    pub catalog: &'static str,
}

/// A payload struct with a versioned packed form.
pub(crate) trait PackedPayload: Sized {
    /// Version written by the encoder.
    const CURRENT_VERSION: u8;
    /// Every version the decoder accepts, including historical ones.
    const SUPPORTED_VERSIONS: &'static [u8];
    /// Fields in encoding order.
    const FIELDS: &'static [FieldSpec];

    fn write_current(&self, w: &mut Writer) -> Result<(), SignalsError>;

    fn read_version(version: u8, r: &mut Reader<'_>) -> Option<Result<Self, SignalsError>>;
}

/// Defines a payload struct and its version-1 codec from one field list.
///
/// Fields encode in order. The struct, its encoder, its decoder and its [`FieldSpec`] table all
/// come from the same list, so they cannot disagree.
macro_rules! payload_struct {
    (
        $(#[$meta:meta])*
        $name:ident {
            $($(#[$fmeta:meta])* $field:ident : $ty:ty => $catalog:literal,)+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name {
            $($(#[$fmeta])* pub $field: $ty,)+
        }

        impl $name {
            fn write_v1(&self, w: &mut Writer) -> Result<(), SignalsError> {
                $(Pack::pack(&self.$field, w)?;)+
                Ok(())
            }

            fn read_v1(r: &mut Reader<'_>) -> Result<Self, SignalsError> {
                Ok(Self { $($field: <$ty as Pack>::unpack(r)?,)+ })
            }
        }

        impl PackedPayload for $name {
            const CURRENT_VERSION: u8 = 1;
            const SUPPORTED_VERSIONS: &'static [u8] = &[1];
            const FIELDS: &'static [FieldSpec] = &[
                $(FieldSpec { name: stringify!($field), catalog: $catalog },)+
            ];

            fn write_current(&self, w: &mut Writer) -> Result<(), SignalsError> {
                self.write_v1(w)
            }

            fn read_version(version: u8, r: &mut Reader<'_>) -> Option<Result<Self, SignalsError>> {
                match version {
                    1 => Some(Self::read_v1(r)),
                    _ => None,
                }
            }
        }
    };
}

payload_struct! {
    /// `VolumeChanged`. At most 11 bytes: version, from, to, gesture, input (1, or 2 for a
    /// remote), section, then two 2-byte times.
    VolumePayload {
        /// Volume before the gesture.
        from: Volume => "from",
        /// Volume after the gesture.
        to: Volume => "to",
        /// Gesture duration.
        gesture: GestureTime => "gesture_ms",
        /// What drove the change.
        input: InputSource => "input",
        /// Section playing at the end of the gesture.
        section: Section => "section",
        /// Time since the track started.
        since_track_start: ElapsedDs => "ms_since_track_start",
        /// Time since the current section started.
        since_section_start: ElapsedDs => "ms_since_section_start",
    }
}

payload_struct! {
    /// `SystemVolumeChanged`: a [`VolumePayload`] plus the endpoint class. At most 12 bytes.
    SystemVolumePayload {
        /// Volume before the change.
        from: Volume => "from",
        /// Volume after the change.
        to: Volume => "to",
        /// Duration of the (coalesced) change.
        gesture: GestureTime => "gesture_ms",
        /// What drove the change (usually a media key or the OS mixer).
        input: InputSource => "input",
        /// Section playing at the end of the change.
        section: Section => "section",
        /// Time since the track started.
        since_track_start: ElapsedDs => "ms_since_track_start",
        /// Time since the current section started.
        since_section_start: ElapsedDs => "ms_since_section_start",
        /// Class of the endpoint whose volume changed.
        endpoint_class: OutputClass => "endpoint_class",
    }
}

payload_struct! {
    /// `MuteToggled`.
    MutePayload {
        /// Whether audio is now muted.
        muted: bool => "muted",
        /// What drove the change.
        input: InputSource => "input",
    }
}

payload_struct! {
    /// `RateChanged`.
    RatePayload {
        /// Rate before the gesture.
        from: Rate => "from",
        /// Rate after the gesture.
        to: Rate => "to",
        /// What drove the change.
        input: InputSource => "input",
        /// Section playing at the end of the gesture.
        section: Section => "section",
    }
}

payload_struct! {
    /// `Skipped`. The position is the event's `pos_ds` column.
    SkipPayload {
        /// How far into the track the skip happened.
        fraction: Fraction => "fraction",
        /// What drove the skip.
        input: InputSource => "input",
        /// Section playing when skipped.
        section: Section => "section",
        /// Where the skipped queue entry came from.
        queue_origin: QueueOrigin => "queue_origin",
    }
}

payload_struct! {
    /// `Completed`.
    CompletedPayload {
        /// Time actually listened (wall time not paused), deciseconds.
        listened_ds: PositionDs => "listened_ms",
        /// Time spent paused during the play, deciseconds.
        paused_ds: PositionDs => "paused_ms",
    }
}

payload_struct! {
    /// `SeekForward` and `SeekBack`.
    SeekPayload {
        /// Position the seek started from, deciseconds.
        from_ds: PositionDs => "from_ms",
        /// Position the seek landed on, deciseconds.
        to_ds: PositionDs => "to_ms",
        /// Section before the seek.
        section_from: Section => "section_from",
        /// Section after the seek.
        section_to: Section => "section_to",
        /// What drove the seek.
        input: InputSource => "input",
    }
}

payload_struct! {
    /// `Replayed`.
    ReplayedPayload {
        /// Loops completed so far; the emitter saturates at 255.
        loop_count: u8 => "loop_count",
    }
}

payload_struct! {
    /// `RepeatModeChanged`.
    RepeatModePayload {
        /// The new mode.
        mode: RepeatMode => "mode",
    }
}

payload_struct! {
    /// `Paused`.
    PausedPayload {
        /// What drove the pause.
        input: InputSource => "input",
    }
}

payload_struct! {
    /// `Resumed`.
    ResumedPayload {
        /// How long playback was paused, deciseconds.
        pause_ds: PositionDs => "pause_ms",
        /// What drove the resume.
        input: InputSource => "input",
    }
}

payload_struct! {
    /// `QueuedNext` and `QueuedLater`.
    QueuedPayload {
        /// The view the user queued from.
        from_view: View => "from_view",
    }
}

payload_struct! {
    /// `RemovedFromQueue`.
    RemovedFromQueuePayload {
        /// Whether the removed entry was a radio pick.
        was_radio_pick: bool => "was_radio_pick",
    }
}

payload_struct! {
    /// `QueueReordered`.
    QueueReorderedPayload {
        /// Signed move distance in queue positions (negative = earlier); the emitter saturates.
        distance: i16 => "distance",
    }
}

payload_struct! {
    /// `PlayStarted`.
    PlayStartedPayload {
        /// Why the play started.
        origin: PlayOrigin => "origin",
        /// What drove it.
        input: InputSource => "input",
    }
}

payload_struct! {
    /// `Rated`.
    RatedPayload {
        /// New rating, 0 (cleared) to 5.
        stars: Stars => "stars",
    }
}

payload_struct! {
    /// `PlaylistAdded` and `PlaylistRemoved`.
    PlaylistEditPayload {
        /// The signals.db dictionary id of the playlist's UUID (the same dictionary as
        /// recording/album/artist ids, `docs/rules/database.md` "Dictionaries"), not a
        /// library.db rowid.
        playlist_id: u32 => "playlist_id",
        /// Kind of playlist.
        playlist_kind: PlaylistKind => "playlist_kind",
    }
}

payload_struct! {
    /// `MetadataEdited`.
    MetadataEditedPayload {
        /// Which tag fields changed.
        fields: MetadataFields => "fields",
    }
}

payload_struct! {
    /// `SearchPerformed`.
    SearchPerformedPayload {
        /// Query length in characters; the emitter saturates at 255.
        query_len: u8 => "query_len",
        /// 32-bit hash of the normalized query.
        query_hash: Hash32 => "query_hash",
        /// Number of results; the emitter saturates at 65 535.
        result_count: u16 => "result_count",
        /// Where the search ran.
        scope: SearchScope => "scope",
    }
}

payload_struct! {
    /// `SearchResultOpened`.
    SearchResultOpenedPayload {
        /// Zero-based rank of the clicked result; the emitter saturates at 255.
        rank: u8 => "rank",
        /// What kind of entity was opened.
        entity_kind: EntityKind => "entity_kind",
    }
}

payload_struct! {
    /// `SearchNoResult`. The query text is not in the event; the hash resolves to it through
    /// the local search history when the user keeps one.
    SearchNoResultPayload {
        /// 32-bit hash of the normalized query.
        query_hash: Hash32 => "query",
    }
}

payload_struct! {
    /// `ViewOpened`.
    ViewOpenedPayload {
        /// The view that was opened.
        view: View => "view",
    }
}

payload_struct! {
    /// `WindowState`.
    WindowStatePayload {
        /// The new window state.
        state: WindowStateKind => "state",
    }
}

payload_struct! {
    /// `OutputDeviceChanged`.
    OutputDevicePayload {
        /// Class of the new default output, derived from transport and form factor.
        class: OutputClass => "class",
        /// How the endpoint is attached.
        transport: OutputTransport => "transport",
        /// The raw OS endpoint form-factor value, stored as-is in one byte. On Windows this is
        /// `EndpointFormFactor`, where `0` is `RemoteNetworkDevice` and `UnknownFormFactor` is
        /// `10`: `0` does not mean unknown.
        form_factor: u8 => "form_factor",
        /// 32-bit salted hash of the device identity; never the device name.
        device_hash: Hash32 => "device_hash",
    }
}

payload_struct! {
    /// `EqChanged`.
    EqPayload {
        /// Preset id (an index into the preset list; 0 = custom/flat).
        preset: u8 => "preset",
        /// Bass band gain in whole dB.
        bass_db: i8 => "bass_db",
        /// Treble band gain in whole dB.
        treble_db: i8 => "treble_db",
    }
}

payload_struct! {
    /// `ExclusiveModeToggled`.
    ExclusiveModePayload {
        /// Whether exclusive / bit-perfect mode is now on.
        on: bool => "on",
    }
}

payload_struct! {
    /// `CrossfadeChanged`.
    CrossfadePayload {
        /// Crossfade length in deciseconds (0 = off); saturates at 25.5 s.
        crossfade_ds: u8 => "ms",
    }
}

payload_struct! {
    /// `SessionStarted`.
    SessionStartedPayload {
        /// How the first play of the session started.
        start_origin: PlayOrigin => "start_origin",
    }
}

payload_struct! {
    /// `SessionEnded`.
    SessionEndedPayload {
        /// Tracks played in the session; the emitter saturates at 65 535.
        tracks: u16 => "tracks",
        /// Session length in minutes; the emitter saturates at 65 535.
        minutes: u16 => "minutes",
    }
}

payload_struct! {
    /// `RadioFeedback`.
    RadioFeedbackPayload {
        /// The correction given.
        kind: RadioFeedbackKind => "kind",
        /// Context dimensions the correction applies to.
        context_scope: ContextScope => "context_scope",
    }
}

payload_struct! {
    /// `ExplanationOpened`.
    ExplanationOpenedPayload {
        /// Id of the explanation reason; the catalog of ids lives with the explanation templates.
        reason_kind: u8 => "reason_kind",
    }
}

/// Defines [`SignalPayload`]: one variant per raw-event kind, so a kind and its payload type
/// can never disagree.
macro_rules! signal_payloads {
    (
        data { $($dv:ident($dt:ty),)* }
        unit { $($uv:ident,)* }
    ) => {
        /// The payload of one raw event; the variant is the [`SignalKind`].
        ///
        /// Kinds without fields (`Favorited`, ...) are unit variants and encode to `None`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum SignalPayload {
            $(
                #[doc = concat!("Payload of [`SignalKind::", stringify!($dv), "`].")]
                $dv($dt),
            )*
            $(
                #[doc = concat!("[`SignalKind::", stringify!($uv), "`] has no fields.")]
                $uv,
            )*
        }

        impl SignalPayload {
            /// The kind this payload belongs to.
            #[must_use]
            pub const fn kind(&self) -> SignalKind {
                match self {
                    $(Self::$dv(_) => SignalKind::$dv,)*
                    $(Self::$uv => SignalKind::$uv,)*
                }
            }

            /// Encodes the payload: `None` for kinds without fields, otherwise the version
            /// byte followed by the fields, at most [`MAX_PAYLOAD_LEN`](crate::MAX_PAYLOAD_LEN)
            /// bytes.
            ///
            /// # Errors
            /// [`SignalsError::PayloadTooLong`] if the layout would exceed the budget (a bug,
            /// caught by tests for every kind).
            pub fn encode(&self) -> Result<Option<EncodedPayload>, SignalsError> {
                match self {
                    $(
                        Self::$dv(payload) => {
                            let mut w = Writer::new();
                            w.byte(<$dt as PackedPayload>::CURRENT_VERSION)?;
                            payload.write_current(&mut w)?;
                            w.finish().map(Some)
                        }
                    )*
                    $(Self::$uv => Ok(None),)*
                }
            }

            /// Decodes the `payload` column of a `signal_event` row of the given kind.
            ///
            /// Accepts every payload version in the kind's `SUPPORTED_VERSIONS`.
            ///
            /// # Errors
            /// A [`SignalsError`] if the kind is not a raw-event kind, the payload is missing or
            /// unexpected, the version is unknown, or the bytes are malformed.
            pub fn decode(kind: SignalKind, bytes: Option<&[u8]>) -> Result<Self, SignalsError> {
                match kind {
                    $(
                        SignalKind::$dv => {
                            let bytes = bytes.ok_or(SignalsError::MissingPayload(kind))?;
                            let mut r = Reader::new(bytes);
                            let version = r.byte()?;
                            let payload = <$dt as PackedPayload>::read_version(version, &mut r)
                                .ok_or(SignalsError::UnsupportedVersion { kind, version })??;
                            r.finish()?;
                            Ok(Self::$dv(payload))
                        }
                    )*
                    $(
                        SignalKind::$uv => match bytes {
                            None => Ok(Self::$uv),
                            Some(_) => Err(SignalsError::UnexpectedPayload(kind)),
                        },
                    )*
                    other => Err(SignalsError::NotAnEvent(other)),
                }
            }
        }

        impl SignalKind {
            /// Fields of the kind's payload in encoding order; empty for kinds without a
            /// packed payload.
            #[must_use]
            pub const fn payload_fields(self) -> &'static [FieldSpec] {
                match self {
                    $(Self::$dv => <$dt as PackedPayload>::FIELDS,)*
                    _ => &[],
                }
            }

            /// Version the encoder writes for this kind; `None` for kinds without a packed
            /// payload.
            #[must_use]
            pub const fn payload_version(self) -> Option<u8> {
                match self {
                    $(Self::$dv => Some(<$dt as PackedPayload>::CURRENT_VERSION),)*
                    _ => None,
                }
            }

            /// Every payload version the decoder accepts for this kind (historical versions
            /// included); empty for kinds without a packed payload.
            #[must_use]
            pub const fn supported_payload_versions(self) -> &'static [u8] {
                match self {
                    $(Self::$dv => <$dt as PackedPayload>::SUPPORTED_VERSIONS,)*
                    _ => &[],
                }
            }
        }
    };
}

signal_payloads! {
    data {
        VolumeChanged(VolumePayload),
        SystemVolumeChanged(SystemVolumePayload),
        MuteToggled(MutePayload),
        RateChanged(RatePayload),
        Skipped(SkipPayload),
        Completed(CompletedPayload),
        SeekForward(SeekPayload),
        SeekBack(SeekPayload),
        Replayed(ReplayedPayload),
        RepeatModeChanged(RepeatModePayload),
        Paused(PausedPayload),
        Resumed(ResumedPayload),
        QueuedNext(QueuedPayload),
        QueuedLater(QueuedPayload),
        RemovedFromQueue(RemovedFromQueuePayload),
        QueueReordered(QueueReorderedPayload),
        PlayStarted(PlayStartedPayload),
        Rated(RatedPayload),
        PlaylistAdded(PlaylistEditPayload),
        PlaylistRemoved(PlaylistEditPayload),
        MetadataEdited(MetadataEditedPayload),
        SearchPerformed(SearchPerformedPayload),
        SearchResultOpened(SearchResultOpenedPayload),
        SearchNoResult(SearchNoResultPayload),
        ViewOpened(ViewOpenedPayload),
        WindowState(WindowStatePayload),
        OutputDeviceChanged(OutputDevicePayload),
        EqChanged(EqPayload),
        ExclusiveModeToggled(ExclusiveModePayload),
        CrossfadeChanged(CrossfadePayload),
        SessionStarted(SessionStartedPayload),
        SessionEnded(SessionEndedPayload),
        RadioFeedback(RadioFeedbackPayload),
        ExplanationOpened(ExplanationOpenedPayload),
    }
    unit {
        PreviousPressed,
        Favorited,
        Unfavorited,
        ArtworkChanged,
    }
}
