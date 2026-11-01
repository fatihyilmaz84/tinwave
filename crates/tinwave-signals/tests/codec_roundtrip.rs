//! Property tests for the payload codec: round trip, size budget, empty = `None`, strictness.

// Test code: a failed unwrap is the failure report. clippy.toml only exempts `#[test]` bodies, not
// helpers in integration-test crates.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;
use proptest::strategy::Union;
use proptest::test_runner::{Config, TestRunner};
use tinwave_signals::*;

fn volume() -> impl Strategy<Value = Volume> {
    any::<u8>().prop_map(Volume::from_raw)
}

fn fraction() -> impl Strategy<Value = Fraction> {
    any::<u8>().prop_map(Fraction::from_raw)
}

fn rate() -> impl Strategy<Value = Rate> {
    any::<u8>().prop_map(Rate::from_raw)
}

fn gesture() -> impl Strategy<Value = GestureTime> {
    any::<u8>().prop_map(GestureTime::from_raw)
}

fn elapsed() -> impl Strategy<Value = ElapsedDs> {
    prop_oneof![9 => any::<u16>(), 1 => Just(u16::MAX)].prop_map(ElapsedDs::from_ds)
}

fn position() -> impl Strategy<Value = PositionDs> {
    prop_oneof![9 => 0..=PositionDs::MAX, 1 => Just(PositionDs::MAX)]
        .prop_map(|ds| PositionDs::new(ds).unwrap())
}

fn input() -> impl Strategy<Value = InputSource> {
    prop_oneof![
        Just(InputSource::Mouse),
        Just(InputSource::Keyboard),
        Just(InputSource::MediaKey),
        Just(InputSource::Tray),
        Just(InputSource::MiniPlayer),
        Just(InputSource::Auto),
        Just(InputSource::System),
        any::<u8>().prop_map(InputSource::Remote),
    ]
}

fn section() -> impl Strategy<Value = Section> {
    (
        proptest::sample::select(SectionKind::ALL),
        0..=Section::MAX_INDEX,
    )
        .prop_map(|(kind, index)| Section::new(kind, index).unwrap())
}

fn choice<T: Copy + std::fmt::Debug + 'static>(all: &'static [T]) -> impl Strategy<Value = T> {
    proptest::sample::select(all)
}

fn hash() -> impl Strategy<Value = Hash32> {
    any::<u32>().prop_map(Hash32)
}

fn wide_u32() -> impl Strategy<Value = u32> {
    prop_oneof![9 => any::<u32>(), 1 => Just(u32::MAX)]
}

fn wide_u16() -> impl Strategy<Value = u16> {
    prop_oneof![9 => any::<u16>(), 1 => Just(u16::MAX)]
}

fn metadata_fields() -> impl Strategy<Value = MetadataFields> {
    any::<u16>()
        .prop_map(|bits| MetadataFields::from_bits(bits & MetadataFields::ALL_BITS).unwrap())
}

fn context_scope() -> impl Strategy<Value = ContextScope> {
    any::<u16>().prop_map(|bits| ContextScope::from_bits(bits & ContextScope::ALL_BITS).unwrap())
}

fn volume_payload() -> impl Strategy<Value = VolumePayload> {
    (
        volume(),
        volume(),
        gesture(),
        input(),
        section(),
        elapsed(),
        elapsed(),
    )
        .prop_map(
            |(from, to, gesture, input, section, since_track_start, since_section_start)| {
                VolumePayload {
                    from,
                    to,
                    gesture,
                    input,
                    section,
                    since_track_start,
                    since_section_start,
                }
            },
        )
}

fn seek_payload() -> impl Strategy<Value = SeekPayload> {
    (position(), position(), section(), section(), input()).prop_map(
        |(from_ds, to_ds, section_from, section_to, input)| SeekPayload {
            from_ds,
            to_ds,
            section_from,
            section_to,
            input,
        },
    )
}

fn queued_payload() -> impl Strategy<Value = QueuedPayload> {
    choice(View::ALL).prop_map(|from_view| QueuedPayload { from_view })
}

fn playlist_payload() -> impl Strategy<Value = PlaylistEditPayload> {
    (wide_u32(), choice(PlaylistKind::ALL)).prop_map(|(playlist_id, playlist_kind)| {
        PlaylistEditPayload {
            playlist_id,
            playlist_kind,
        }
    })
}

/// A strategy for the payloads of one raw-event kind; `None` for counter and derived kinds.
///
/// The match has no wildcard: adding a kind without deciding how to test it fails to compile.
fn strategy_for(kind: SignalKind) -> Option<BoxedStrategy<SignalPayload>> {
    use SignalKind as K;
    let s = match kind {
        K::VolumeChanged => volume_payload()
            .prop_map(SignalPayload::VolumeChanged)
            .boxed(),
        K::SystemVolumeChanged => (volume_payload(), choice(OutputClass::ALL))
            .prop_map(|(v, endpoint_class)| {
                SignalPayload::SystemVolumeChanged(SystemVolumePayload {
                    from: v.from,
                    to: v.to,
                    gesture: v.gesture,
                    input: v.input,
                    section: v.section,
                    since_track_start: v.since_track_start,
                    since_section_start: v.since_section_start,
                    endpoint_class,
                })
            })
            .boxed(),
        K::MuteToggled => (any::<bool>(), input())
            .prop_map(|(muted, input)| SignalPayload::MuteToggled(MutePayload { muted, input }))
            .boxed(),
        K::RateChanged => (rate(), rate(), input(), section())
            .prop_map(|(from, to, input, section)| {
                SignalPayload::RateChanged(RatePayload {
                    from,
                    to,
                    input,
                    section,
                })
            })
            .boxed(),
        K::Skipped => (fraction(), input(), section(), choice(QueueOrigin::ALL))
            .prop_map(|(fraction, input, section, queue_origin)| {
                SignalPayload::Skipped(SkipPayload {
                    fraction,
                    input,
                    section,
                    queue_origin,
                })
            })
            .boxed(),
        K::Completed => (position(), position())
            .prop_map(|(listened_ds, paused_ds)| {
                SignalPayload::Completed(CompletedPayload {
                    listened_ds,
                    paused_ds,
                })
            })
            .boxed(),
        K::SeekForward => seek_payload().prop_map(SignalPayload::SeekForward).boxed(),
        K::SeekBack => seek_payload().prop_map(SignalPayload::SeekBack).boxed(),
        K::Replayed => any::<u8>()
            .prop_map(|loop_count| SignalPayload::Replayed(ReplayedPayload { loop_count }))
            .boxed(),
        K::RepeatModeChanged => choice(RepeatMode::ALL)
            .prop_map(|mode| SignalPayload::RepeatModeChanged(RepeatModePayload { mode }))
            .boxed(),
        K::Paused => input()
            .prop_map(|input| SignalPayload::Paused(PausedPayload { input }))
            .boxed(),
        K::Resumed => (position(), input())
            .prop_map(|(pause_ds, input)| {
                SignalPayload::Resumed(ResumedPayload { pause_ds, input })
            })
            .boxed(),
        K::QueuedNext => queued_payload().prop_map(SignalPayload::QueuedNext).boxed(),
        K::QueuedLater => queued_payload()
            .prop_map(SignalPayload::QueuedLater)
            .boxed(),
        K::RemovedFromQueue => any::<bool>()
            .prop_map(|was_radio_pick| {
                SignalPayload::RemovedFromQueue(RemovedFromQueuePayload { was_radio_pick })
            })
            .boxed(),
        K::QueueReordered => {
            prop_oneof![9 => any::<i16>(), 1 => Just(i16::MIN), 1 => Just(i16::MAX)]
                .prop_map(|distance| {
                    SignalPayload::QueueReordered(QueueReorderedPayload { distance })
                })
                .boxed()
        }
        K::PlayStarted => (choice(PlayOrigin::ALL), input())
            .prop_map(|(origin, input)| {
                SignalPayload::PlayStarted(PlayStartedPayload { origin, input })
            })
            .boxed(),
        K::Rated => (0..=Stars::MAX)
            .prop_map(|s| {
                SignalPayload::Rated(RatedPayload {
                    stars: Stars::new(s).unwrap(),
                })
            })
            .boxed(),
        K::PlaylistAdded => playlist_payload()
            .prop_map(SignalPayload::PlaylistAdded)
            .boxed(),
        K::PlaylistRemoved => playlist_payload()
            .prop_map(SignalPayload::PlaylistRemoved)
            .boxed(),
        K::MetadataEdited => metadata_fields()
            .prop_map(|fields| SignalPayload::MetadataEdited(MetadataEditedPayload { fields }))
            .boxed(),
        K::SearchPerformed => (any::<u8>(), hash(), wide_u16(), choice(SearchScope::ALL))
            .prop_map(|(query_len, query_hash, result_count, scope)| {
                SignalPayload::SearchPerformed(SearchPerformedPayload {
                    query_len,
                    query_hash,
                    result_count,
                    scope,
                })
            })
            .boxed(),
        K::SearchResultOpened => (any::<u8>(), choice(EntityKind::ALL))
            .prop_map(|(rank, entity_kind)| {
                SignalPayload::SearchResultOpened(SearchResultOpenedPayload { rank, entity_kind })
            })
            .boxed(),
        K::SearchNoResult => hash()
            .prop_map(|query_hash| {
                SignalPayload::SearchNoResult(SearchNoResultPayload { query_hash })
            })
            .boxed(),
        K::ViewOpened => choice(View::ALL)
            .prop_map(|view| SignalPayload::ViewOpened(ViewOpenedPayload { view }))
            .boxed(),
        K::WindowState => choice(WindowStateKind::ALL)
            .prop_map(|state| SignalPayload::WindowState(WindowStatePayload { state }))
            .boxed(),
        K::OutputDeviceChanged => (
            choice(OutputClass::ALL),
            choice(OutputTransport::ALL),
            any::<u8>(),
            hash(),
        )
            .prop_map(|(class, transport, form_factor, device_hash)| {
                SignalPayload::OutputDeviceChanged(OutputDevicePayload {
                    class,
                    transport,
                    form_factor,
                    device_hash,
                })
            })
            .boxed(),
        K::EqChanged => (any::<u8>(), any::<i8>(), any::<i8>())
            .prop_map(|(preset, bass_db, treble_db)| {
                SignalPayload::EqChanged(EqPayload {
                    preset,
                    bass_db,
                    treble_db,
                })
            })
            .boxed(),
        K::ExclusiveModeToggled => any::<bool>()
            .prop_map(|on| SignalPayload::ExclusiveModeToggled(ExclusiveModePayload { on }))
            .boxed(),
        K::CrossfadeChanged => any::<u8>()
            .prop_map(|crossfade_ds| {
                SignalPayload::CrossfadeChanged(CrossfadePayload { crossfade_ds })
            })
            .boxed(),
        K::SessionStarted => choice(PlayOrigin::ALL)
            .prop_map(|start_origin| {
                SignalPayload::SessionStarted(SessionStartedPayload { start_origin })
            })
            .boxed(),
        K::SessionEnded => (wide_u16(), wide_u16())
            .prop_map(|(tracks, minutes)| {
                SignalPayload::SessionEnded(SessionEndedPayload { tracks, minutes })
            })
            .boxed(),
        K::RadioFeedback => (choice(RadioFeedbackKind::ALL), context_scope())
            .prop_map(|(kind, context_scope)| {
                SignalPayload::RadioFeedback(RadioFeedbackPayload {
                    kind,
                    context_scope,
                })
            })
            .boxed(),
        K::ExplanationOpened => any::<u8>()
            .prop_map(|reason_kind| {
                SignalPayload::ExplanationOpened(ExplanationOpenedPayload { reason_kind })
            })
            .boxed(),
        K::PreviousPressed => Just(SignalPayload::PreviousPressed).boxed(),
        K::Favorited => Just(SignalPayload::Favorited).boxed(),
        K::Unfavorited => Just(SignalPayload::Unfavorited).boxed(),
        K::ArtworkChanged => Just(SignalPayload::ArtworkChanged).boxed(),
        K::PerceivedLevel | K::RateSession | K::PageDwell | K::ArtworkHover | K::Impression => {
            return None;
        }
    };
    Some(s)
}

/// Checks every codec property for one payload.
fn check(payload: SignalPayload) -> Result<(), TestCaseError> {
    let kind = payload.kind();
    let encoded = payload
        .encode()
        .map_err(|e| TestCaseError::fail(e.to_string()))?;

    // Empty payloads are NULL; payloads with fields are never empty.
    prop_assert_eq!(kind.payload_fields().is_empty(), encoded.is_none());

    let bytes = encoded.as_ref().map(EncodedPayload::as_bytes);
    if let Some(bytes) = bytes {
        prop_assert!(
            bytes.len() <= MAX_PAYLOAD_LEN,
            "{kind}: {} bytes",
            bytes.len()
        );
        prop_assert_eq!(Some(bytes[0]), kind.payload_version());
    }

    let decoded =
        SignalPayload::decode(kind, bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
    prop_assert_eq!(decoded, payload);
    Ok(())
}

fn any_payload() -> impl Strategy<Value = SignalPayload> {
    let all: Vec<BoxedStrategy<SignalPayload>> = SignalKind::ALL
        .iter()
        .filter_map(|k| strategy_for(*k))
        .collect();
    Union::new(all)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn any_payload_round_trips_within_budget(payload in any_payload()) {
        check(payload)?;
    }
}

#[test]
fn every_event_kind_has_a_strategy_and_round_trips() {
    let mut covered = 0;
    for kind in SignalKind::ALL {
        let strategy = strategy_for(*kind);
        assert_eq!(strategy.is_some(), kind.is_event(), "{kind}");
        let Some(strategy) = strategy else { continue };
        let mut runner = TestRunner::new(Config::with_cases(512));
        runner
            .run(&strategy, |payload| {
                prop_assert_eq!(payload.kind(), *kind);
                check(payload)
            })
            .unwrap_or_else(|e| panic!("{kind}: {e}"));
        covered += 1;
    }
    assert_eq!(covered, SignalKind::events().count());
}

/// All-maximum values for every kind whose size depends on the data.
fn worst_cases() -> Vec<(SignalPayload, usize)> {
    let max_section = Section::new(SectionKind::Unknown, Section::MAX_INDEX).unwrap();
    let remote = InputSource::Remote(u8::MAX);
    let volume = VolumePayload {
        from: Volume::MAX,
        to: Volume::MAX,
        gesture: GestureTime::from_raw(u8::MAX),
        input: remote,
        section: max_section,
        since_track_start: ElapsedDs::from_ds(u16::MAX),
        since_section_start: ElapsedDs::from_ds(u16::MAX),
    };
    let seek = SeekPayload {
        from_ds: PositionDs::new(PositionDs::MAX).unwrap(),
        to_ds: PositionDs::new(PositionDs::MAX).unwrap(),
        section_from: max_section,
        section_to: max_section,
        input: remote,
    };
    vec![
        (SignalPayload::VolumeChanged(volume), 11),
        (
            SignalPayload::SystemVolumeChanged(SystemVolumePayload {
                from: volume.from,
                to: volume.to,
                gesture: volume.gesture,
                input: volume.input,
                section: volume.section,
                since_track_start: volume.since_track_start,
                since_section_start: volume.since_section_start,
                endpoint_class: OutputClass::Unknown,
            }),
            12,
        ),
        (SignalPayload::SeekForward(seek), 11),
        (SignalPayload::SeekBack(seek), 11),
        (
            SignalPayload::Completed(CompletedPayload {
                listened_ds: PositionDs::new(PositionDs::MAX).unwrap(),
                paused_ds: PositionDs::new(PositionDs::MAX).unwrap(),
            }),
            7,
        ),
        (
            SignalPayload::Resumed(ResumedPayload {
                pause_ds: PositionDs::new(PositionDs::MAX).unwrap(),
                input: remote,
            }),
            6,
        ),
        (
            SignalPayload::PlaylistAdded(PlaylistEditPayload {
                playlist_id: u32::MAX,
                playlist_kind: PlaylistKind::Smart,
            }),
            7,
        ),
        (
            SignalPayload::SearchPerformed(SearchPerformedPayload {
                query_len: u8::MAX,
                query_hash: Hash32(u32::MAX),
                result_count: u16::MAX,
                scope: SearchScope::Playlists,
            }),
            10,
        ),
        (
            SignalPayload::SessionEnded(SessionEndedPayload {
                tracks: u16::MAX,
                minutes: u16::MAX,
            }),
            7,
        ),
        (
            SignalPayload::QueueReordered(QueueReorderedPayload { distance: i16::MIN }),
            4,
        ),
        (
            SignalPayload::MetadataEdited(MetadataEditedPayload {
                fields: MetadataFields::from_bits(MetadataFields::ALL_BITS).unwrap(),
            }),
            3,
        ),
    ]
}

#[test]
fn worst_case_payloads_fit_the_twelve_byte_budget() {
    for (payload, expected_len) in worst_cases() {
        let encoded = payload.encode().unwrap().unwrap();
        assert_eq!(encoded.len(), expected_len, "{}", payload.kind());
        assert!(encoded.len() <= MAX_PAYLOAD_LEN, "{}", payload.kind());
    }
}

#[test]
fn kinds_without_fields_encode_to_none() {
    for payload in [
        SignalPayload::PreviousPressed,
        SignalPayload::Favorited,
        SignalPayload::Unfavorited,
        SignalPayload::ArtworkChanged,
    ] {
        assert_eq!(payload.encode(), Ok(None));
        assert!(payload.kind().payload_fields().is_empty());
        assert_eq!(payload.kind().payload_version(), None);
        assert_eq!(SignalPayload::decode(payload.kind(), None), Ok(payload));
        assert_eq!(
            SignalPayload::decode(payload.kind(), Some(&[1])),
            Err(SignalsError::UnexpectedPayload(payload.kind()))
        );
    }
}

#[test]
fn every_kind_with_fields_has_a_version_and_a_non_empty_encoding() {
    let mut with_fields = 0;
    for kind in SignalKind::events() {
        let has_fields = !kind.payload_fields().is_empty();
        assert_eq!(kind.payload_version().is_some(), has_fields, "{kind}");
        if has_fields {
            with_fields += 1;
            assert_eq!(
                SignalPayload::decode(kind, None),
                Err(SignalsError::MissingPayload(kind))
            );
        }
    }
    assert_eq!(with_fields, 34);
}

#[test]
fn counter_and_derived_kinds_have_no_payload() {
    for kind in SignalKind::ALL.iter().filter(|k| !k.is_event()) {
        assert_eq!(
            SignalPayload::decode(*kind, None),
            Err(SignalsError::NotAnEvent(*kind))
        );
        assert!(kind.payload_fields().is_empty());
        assert_eq!(kind.payload_version(), None);
        assert!(kind.supported_payload_versions().is_empty());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8192))]

    /// Random bytes never panic the decoder, and anything it accepts re-encodes to the same
    /// bytes (the encoding is canonical: there is exactly one byte form per value).
    #[test]
    fn decoder_is_total_and_canonical(
        kind_index in 0..SignalKind::ALL.len(),
        bytes in proptest::collection::vec(any::<u8>(), 0..=14),
    ) {
        let kind = SignalKind::ALL[kind_index];
        // Bias towards the current version byte so the field decoders are exercised.
        let mut bytes = bytes;
        if let (Some(version), Some(first)) = (kind.payload_version(), bytes.first_mut()) {
            if *first & 1 == 0 {
                *first = version;
            }
        }
        if let Ok(payload) = SignalPayload::decode(kind, Some(&bytes)) {
            let encoded = payload.encode().unwrap().unwrap();
            prop_assert_eq!(encoded.as_bytes(), &bytes[..]);
        }
    }
}
