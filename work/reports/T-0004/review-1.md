REVIEW T-0004 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass, clippy pass (--workspace --all-targets -D warnings), tests 89 passed / 0 failed (39 core + 29 signals unit + 8 catalog_sync + 7 codec_roundtrip + 6 payload_versions), deny pass (advisories ok, bans ok, licenses ok, sources ok, no warnings)

acceptance criteria:
- [x] Every E-class catalog signal has a kind, tier and payload, compared to a fixture derived from the catalog. Verified by `tests/catalog_sync.rs`, which checks kind-set equality both ways, class, tier and field sets. I ran mutations on a scratch copy (git archive of HEAD):
  - catalog `Rated` T0 -> T1 fails `storage_class_and_tier_match_the_catalog`;
  - adding a field to `Completed` in the catalog fails `payload_fields_match_the_catalog`.
  - 38 E kinds, 3 C kinds and 2 D kinds match the catalog. `SignalDropped` (meta) has no kind.
- [x] Discriminants are stable: `kind::tests::discriminants_are_frozen` (kind.rs:276-332) compares the full (name, number) table, so changing a value fails. Uniqueness, non-zero and <128 are also checked.
- [x] Codec round-trips all payloads with proptest: `codec_roundtrip.rs` runs `any_payload_round_trips_within_budget` and `every_event_kind_has_a_strategy_and_round_trips`. `strategy_for` has no wildcard, so a new kind won't compile without a strategy.
- [x] Encoded payload ≤ 12 bytes for every kind, and empty payloads encode to `None`:
  - `Writer` has a fixed 12-byte buffer (codec.rs:66-104) and returns `PayloadTooLong` rather than overflowing.
  - `check()` asserts len ≤ 12 and that empty fields mean `None`, for every case.
  - `worst_case_payloads_fit_the_twelve_byte_budget` pins the worst cases. I re-derived them by hand: SystemVolume 12, Volume/Seek 11, SearchPerformed 10.
  - `kinds_without_fields_encode_to_none` passes.
- [x] Decoder accepts every historical payload_version (only v1 now):
  - `payload_versions.rs` has hand-written v1 bytes for all 38 kinds. I re-derived the SystemVolume, SeekBack and SearchPerformed bytes by hand.
  - `every_supported_version_of_every_kind_has_a_fixture` and `unknown_versions_are_rejected_not_guessed` pass.
  - The version is handled per payload type (payload.rs:84-89, 484-505).

Other points checked and OK:
- **Hostile bytes:** `Reader` never allocates and never indexes out of bounds (`get` plus `ok_or`). A huge blob reads at most ~12 bytes, then returns `TrailingBytes`.
- **Varints:** capped at 5 bytes, with a 4-bit check on the 5th byte (codec.rs:124-142). Non-canonical forms are rejected. u16 and PositionDs (2^21-1, at most 3 bytes) are range-checked, and zigzag is bijective, so the encoding is canonical. `decoder_is_total_and_canonical` (8192 random cases) passes.
- **Quantization:** float inputs reject NaN and infinity, then clamp before the cast. Rate returns an error when out of range; that is explicit, not a silent wrap. ms constructors saturate. serde paths validate Section, PositionDs, Stars and flags.
- **Tiers:** WindowState and OutputDeviceChanged are T1, everything else is T0. The Q9 rule is implemented: `tier()` takes the highest tier of `derived_inputs()` (kind.rs:182-218), so PerceivedLevel is T1 and RateSession is T0. Both the unit test and catalog_sync assert it. No T2/T3 kinds exist.
- **Q4:** the `playlist_id` doc is at payload.rs:280-282.
- **Dependencies:** runtime deps are tinwave-core, serde and thiserror only (`cargo tree -e normal`). proptest and serde_json are dev-only. The lockfile additions are only proptest's tree plus tinwave-signals.
- **Scope:** `git diff main...HEAD --stat` touches only crates/tinwave-signals/, Cargo.toml (2 lines, both in [workspace.dependencies]) and Cargo.lock. docs/guides/signal-catalog.md is identical to main. No work/ edits, no TODOs.

blocking findings:
- crates/tinwave-signals/src/enums.rs:478-516 — `enum_codes_are_frozen` only compares the list of codes in declaration order, never which name has which code. These codes are persisted inside payloads.
  - Failure scenario, verified by mutation: I changed `QueueOrigin` to `Playlist = 2, Album = 3` (they were `Album = 2, Playlist = 3`). All 50 signals tests still pass, and every stored `Skipped` byte 2/3 silently changes meaning.
  - The same hole exists for every coded enum. A variant inserted mid-list with renumbering also looks like a harmless append ([0..n] becomes [0..n+1]).
  - The handoff cites this test as proof that the codes are frozen.
  - Fix: freeze (name, code) pairs per enum, as kind.rs FROZEN does, including the InputSource tags (Keyboard..MiniPlayer are only pinned indirectly) and the flag bit positions per name.
- crates/tinwave-signals/tests/catalog_sync.rs:321-346 — `enum_values_listed_in_payload_columns_match_the_code` only checks the 4 hard-coded fields. It silently ignores a value list the catalog adds for any other field.
  - The assertion message says "every enum with a value list in the catalog must be checked", but the code only checks the reverse direction (that every expected key appears in the catalog).
  - Failure scenario, verified by mutation: I added `` `mode` (`Off|All|Once`) `` to the catalog. All tests pass even though the code has `One`, not `Once`. P-0011 is about to add exactly these lists (View, PlaylistKind, SearchScope, EntityKind, WindowStateKind, RadioFeedbackKind), so the guard would not cover them.
  - Fix: fail on any field in `row.enum_values` that is neither in `expected` nor explicitly handled elsewhere (`input`). Pre-map the Q5 enums (`mode`, `from_view`/`view`, `playlist_kind`, `scope`, `entity_kind`, `state`, `kind`) so P-0011 is checked the moment it lands.

design check: n/a

non-blocking findings:
- crates/tinwave-signals/tests/catalog_sync.rs:111-114 — columns are positional (`cells[1]` = tier, `cells[3]` = payload), even though the header row is read. Inserting a column (e.g. a per-signal Class or bytes/day column) fails 3 tests with misleading messages (verified by mutation). It fails loudly, not silently. Look up columns by header name.
- crates/tinwave-signals/tests/catalog_sync.rs:35 — `TYPE_WORDS` only knows u8/u16/u32. Writing `` `bool` ``, `` `i16` `` or `` `u64` `` in a payload cell turns it into a phantom field. This is a loud but brittle failure.
- crates/tinwave-signals/tests/catalog_sync.rs:292-300 — multi-kind rows are checked only as a union. If `Paused` lost `input`, the test would still pass because `Resumed` has it.
- crates/tinwave-signals/src/payload.rs:353 — the `form_factor` doc says "`0` = unknown". In Windows `EndpointFormFactor`, 0 is `RemoteNetworkDevice` and `UnknownFormFactor` is 10. An emitter that follows this doc would conflate the two. Document it as the raw OS value instead.
- crates/tinwave-signals/src/kind.rs:36-37 — "retired kinds keep theirs" is only a convention. If someone deletes a variant together with its FROZEN row, a later kind can reuse that number and all tests pass.

proposals:
- P: Add a RETIRED discriminant/code list checked by the frozen tests | why: enforce "never reused" across deletions, not just within the current enum | size: S
- P: Header-driven column lookup in the catalog_sync parser | why: lets catalog tables gain columns (bytes/day, class) without breaking the guard | size: S

notes for lead:
- Both blocking items are test-only fixes in crates/tinwave-signals/; no production code or schema change is needed.
- main has moved 2 lead commits past the merge base (6dc383b, 18830b0: work/ and docs/rules/architecture.md). The branch touches neither, so no conflicts are expected.
- The catalog-copy commit ec6479e is a no-op against main.
- The scratch copy for the mutation runs is in the session scratchpad (mut/, script run.sh). Nothing in the repo or worktree was modified.

---
Lead (2026-11-02): round 2 sent to the same builder with blocking 1–2 plus non-blocking items folded in: header-driven
columns (P-0021), per-kind check for multi-kind rows, form_factor doc, RETIRED list (P-0020); TYPE_WORDS optional.
