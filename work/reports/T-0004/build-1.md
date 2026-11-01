HANDOFF T-0004 (round 1)
status: done
branch: t/T-0004-signal-schema-codec   commit: a5f0dd0 (catalog sync commit before it: ec6479e)
summary: New crate `tinwave-signals` with `SignalKind` (43 kinds, frozen discriminants, `StorageClass`, `tier()`), quantized payload structs for all 38 raw-event kinds, a strict versioned codec and `SignalEvent`. `SignalPayload` has one variant per event kind, so a kind and its payload can't disagree. Encoded payloads are at most 12 bytes, and kinds with no fields encode to `None`. I applied your ADR 0004 catalog update: the code and tests follow the updated rows, and the fixture is the catalog itself, parsed at test time.

acceptance criteria:
- [x] Every E-class catalog signal has a kind, tier and payload, compared to a fixture derived from the catalog.
  Proof: `tests/catalog_sync.rs` (8 tests) passes. The "fixture" is the catalog: `include_str!` of docs/guides/signal-catalog.md, parsed at test time.
  - It parses every `Signal | Tier | ...` table row: names, tier, payload field names, enum value lists in `(`A|B|C`)`, and "same as above". It also parses the storage class C sentence and the Common enums bullets.
  - It asserts: kind set equality both ways, storage class, tier, payload fields per row, enum values against the code, and the InputSource and Section lists.
  - Not vacuous: I ran it against the pre-ADR catalog (3 failures: class values, `System`, payload fields) and with `Rated` mutated to T1 (fails). Both were restored afterwards.
- [x] Discriminants are stable. Proof: `kind::tests::discriminants_are_frozen` (literal name-to-number table), plus unique/nonzero/under-128 checks. Enum codes are frozen in `enums::tests::enum_codes_are_frozen`.
- [x] Codec round-trips all payloads with `proptest`. Proof: `tests/codec_roundtrip.rs`.
  - `any_payload_round_trips_within_budget` runs 4096 cases over a union of all event kinds.
  - `every_event_kind_has_a_strategy_and_round_trips` runs 512 cases per kind. `strategy_for` has no wildcard arm, so a new kind won't compile until it has a strategy.
  - `decoder_is_total_and_canonical` runs 8192 cases of random bytes: the decoder never panics and anything it accepts re-encodes to the same bytes.
- [x] Encoded payload at most 12 bytes for every kind; empty payloads encode to `None`.
  - Proof: `check()` asserts len at most 12 and `payload_fields().is_empty() == encoded.is_none()` for every case. `worst_case_payloads_fit_the_twelve_byte_budget` pins exact worst cases: SystemVolumeChanged is 12, VolumeChanged and Seek are 11, SearchPerformed is 10. `kinds_without_fields_encode_to_none` covers the 4 unit kinds.
  - The version byte is counted inside the 12.
- [x] Decoder accepts every historical `payload_version`. Proof: `tests/payload_versions.rs`.
  - Hand-written v1 byte fixtures exist for all 38 kinds, decoded by `decoder_reads_every_fixture_of_every_version`. The encoder is checked against them for the current version.
  - `every_supported_version_of_every_kind_has_a_fixture` fails if a supported version has no fixture. Unknown versions, truncation and trailing bytes are rejected.
  - Per-kind `payload_version()` and `supported_payload_versions()` are exposed.

checks: fmt pass, clippy pass (workspace, all targets, -D warnings), tests 70 passed / 0 failed (workspace: 21 core + 28 unit + 8 catalog_sync + 7 codec_roundtrip + 6 payload_versions), deny pass (advisories, bans, licenses, sources ok; proptest adds rand 0.10, rand_core, rand_xorshift, num-traits, regex-syntax, bitflags, unarray, chacha20)
files changed:
- Cargo.toml, Cargo.lock
- crates/tinwave-signals/Cargo.toml
- crates/tinwave-signals/src/{lib,kind,enums,quant,codec,payload,event,error}.rs
- crates/tinwave-signals/tests/{catalog_sync,codec_roundtrip,payload_versions}.rs
- docs/guides/signal-catalog.md (the out-of-scope copy described below)

signals added/changed:
- Schema only, nothing emitted. Discriminants are grouped 16 per catalog section: Volume 1-4, Speed 17-18, Seek/skip 33-41, Queue/library 49-63, Browsing 65-69, Output 81-84, Session/meta 113-116. They stay under 128 and 0 is unused.
- Tiers are T0 except `WindowState` and `OutputDeviceChanged` (T1).
- Payload bytes per event (version byte included): common 2-6, volume/seek 10-12, worst case 12. This keeps the 60 B/event on-disk target feasible, but the real figure is for the storage ticket to measure.
- No catalog volume-table rows were added, because nothing emits yet.

ui: none
docs updated: docs/guides/signal-catalog.md only, to the rows from main (see deviations)

deviations from ticket:
- Mirrored your ADR 0004 catalog update.
  - The three rows (`SystemVolumeChanged`, `MuteToggled`, `OutputDeviceChanged`) are in the catalog fixture, because the test parses the catalog in the worktree.
  - `InputSource::System` is added, with tag byte 7.
  - `OutputClass` gains `Virtual` and `Unknown`.
  - New `OutputTransport` (Bluetooth|Usb|Hda|Virtual|Other).
  - `OutputDevicePayload` is `class, transport, form_factor: u8, device_hash` (8 bytes).
- Out of scope but needed: I copied main's `docs/guides/signal-catalog.md` byte-for-byte into this branch as a separate commit (ec6479e). Otherwise the sync test fails on the branch, since the branch predates ADR 0004. It merges as a no-op against main; drop that commit if you prefer.
- `SignalKind` also contains the counter kinds (PageDwell, ArtworkHover, Impression) and derived kinds (PerceivedLevel, RateSession), not just E-class. `StorageClass` is per kind as the ticket says, and `ui_counter.kind` needs the counter codes. Only the 38 event kinds have payloads.
- `SignalDropped` (tier "meta", in-memory only) has no kind.
- Workspace dependency `tinwave-core` carries `version = "0.1.0"` because `cargo deny` bans wildcard dependencies.
- The three integration-test files carry `#![allow(clippy::unwrap_used, clippy::expect_used)]`, since clippy.toml only exempts `#[test]` bodies, not helpers.

proposals:
- P: Add `System` to the Common enums `InputSource` bullet in signal-catalog.md | why: the bullet on main lacks `System`; the test currently unions it in from the `SystemVolumeChanged` row | size: S | evidence: docs/guides/signal-catalog.md:15 and the row at :25
- P: Per-kind payload field/size table in the Signal Center and export | why: `SignalKind::payload_fields()` (`FieldSpec` name plus catalog name) is ready to drive JSONL export and the Signal Center list | size: S | evidence: crates/tinwave-signals/src/payload.rs `payload_fields`
- P: Growth-test generator should consume `SignalPayload` and `strategy_for`-style sample builders | why: reuse real encoded sizes instead of estimates | size: M | evidence: docs/rules/testing.md growth test section

questions for lead (the catalog is ambiguous; each choice is documented in code and trivially changeable before storage lands):
1. The catalog lists no payload for `PreviousPressed`, `Favorited`, `Unfavorited` and `ArtworkChanged` except `pos_ds`. They encode to `None`, and `pos_ds` is the event column. Confirm `pos_ds` payload entries mean the column.
2. `InputSource::Remote(device_id)`: a UUID can't fit 12 bytes, so it is `Remote(u8)`, a slot in the local paired-device table. Is a u8 slot acceptable, or do you want a u16 (needs a larger budget on VolumeChanged and SystemVolumeChanged)?
3. `SearchNoResult` catalog payload is `query` (text), but payloads are packed bytes. It stores `query_hash` (u32, `FieldSpec.catalog = "query"`). The text would resolve through the local search history, which is a later ticket. Confirm.
4. `playlist_id` is stored as a u32 varint "signals.db surrogate", since there is no playlist dictionary table in database.md. Which id should the writer map to?
5. Enums the catalog names but doesn't list:
   - `from_view` and `ViewOpened.view` share the `View` enum: Home, Library, Album, Artist, Playlist, Search, Queue, Radio, NowPlaying, Lyrics, MiniPlayer, Insights.
   - `playlist_kind` is Manual|Smart.
   - `scope` is All|Tracks|Albums|Artists|Playlists.
   - `entity_kind` is Track|Album|Artist|Playlist.
   - `state` is Focused|Unfocused|Minimized|Tray.
   - `RadioFeedback.kind` is NotNow|NeverInContext|MoreLikeThis|ForgetThisPlay.
   - `context_scope` is a u16 flag set (DAYPART, WEEKDAY, SEASON, WEATHER, DEVICE, ACTIVITY, PLACE; empty = this play only).
   - `reason_kind` is an opaque u8 until the explanation templates exist.
   - `MetadataEdited.fields` is a u16 flag set of tag names.
   - Please confirm or correct.
6. `Rated.stars` is 0-5 (0 clears). The catalog maps ratings to -2..+2 but doesn't say whole or half stars.
7. `SessionStarted` carries `start_origin` (a PlayOrigin); `SessionEnded` carries `tracks` and `minutes`. The catalog row is shared by both, so this split is my reading.
8. `SystemVolumeChanged.endpoint_class` reuses `OutputClass`, my reading since the catalog doesn't define it.
9. Derived kinds (PerceivedLevel, RateSession) have no tier in the catalog, so the code gives them T0, the tier of their inputs. PerceivedLevel also uses endpoint gain, which could argue for T1.
10. Quantization choices: gesture ms in 50 ms steps (u8, 12.75 s max), "ms since start" in deciseconds (u16, 109 min max), seek and listened positions as deciseconds (varint, 58 h max), and EQ in whole dB. All saturate on construction. Say if you want finer resolution anywhere.
11. Needed from `tinwave-core`: nothing, so no T-0013 dependency. `TzOffsetMin` isn't used because tz lives on the session row.

---
Lead answers (2026-11-01), sent to the builder before review:
- Q9 → change: derived kinds take the highest tier of their inputs; PerceivedLevel = T1, RateSession = T0.
- Q4 → doc comment: `playlist_id` is the signals.db dictionary id of the playlist UUID (database.md "Dictionaries").
- Q1–Q3, Q5–Q8, Q10, Q11 → accepted as built. Q5 enum lists and the derived-tier rule go into the catalog on main after merge (P-0011).

---
HANDOFF T-0004 (round 1, amended)
status: done
branch: t/T-0004-signal-schema-codec   commit: 9bb84ee (earlier: a5f0dd0 crate, ec6479e catalog copy, kept as you asked)
summary: Q9 is now encoded as a rule: a derived kind takes the highest tier among its inputs. PerceivedLevel is T1 and RateSession is T0. Q4 meaning is documented. Nothing else changed.

changes in this amend (crates/tinwave-signals only):
- `src/kind.rs`:
  - New `SignalKind::derived_inputs()`: PerceivedLevel is computed from VolumeChanged, SystemVolumeChanged and OutputDeviceChanged; RateSession from RateChanged.
  - `tier()` is still a const fn. Its doc comment states the rule; it reads the tier from the kind table for non-derived kinds and takes the highest input tier for derived ones.
  - In the kind table the two derived rows now say `Inputs` instead of a hand-written tier.
  - New unit test `derived_kinds_take_the_highest_tier_of_their_inputs`: PerceivedLevel is T1 and RateSession is T0. For every derived kind it also checks that the inputs are raw events and that the tier equals the max of the input tiers. Non-derived kinds must have no inputs.
- `tests/catalog_sync.rs`: for catalog rows marked "derived", the test now asserts tier == max of `derived_inputs()` tiers, instead of a hardcoded T0. The test header doc is updated. It passes with the catalog on main as-is, so the rule sentence you add after merge won't affect it.
- `src/payload.rs`: the `playlist_id` doc now says it is the signals.db dictionary id of the playlist's UUID (same dictionary as recording/album/artist ids, database.md "Dictionaries"), not a library.db rowid.

checks: fmt pass, clippy pass (workspace, all targets, -D warnings), tests 71 passed / 0 failed (21 core + 29 unit + 8 catalog_sync + 7 codec_roundtrip + 6 payload_versions), deny pass (advisories, bans, licenses, sources ok)
files changed in this amend: crates/tinwave-signals/src/kind.rs, crates/tinwave-signals/src/payload.rs, crates/tinwave-signals/tests/catalog_sync.rs
signals added/changed: none beyond the earlier handoff; the only schema change is PerceivedLevel's tier, T0 to T1.
docs updated: none (catalog still needs your rule sentence after merge)
deviations from ticket: none new
proposals: none new
questions for lead: none
