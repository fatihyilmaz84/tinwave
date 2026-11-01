REVIEW T-0004 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy pass (also re-run cold on a fresh scratch copy of b13237e with -D warnings: clean), tests 93 passed / 0 failed (39 core + 33 signals unit + 8 catalog_sync + 7 codec_roundtrip + 6 payload_versions), deny pass (advisories ok, bans ok, licenses ok, sources ok)

acceptance criteria:
- [x] Every E-class catalog signal has a kind, tier and payload, checked against a fixture derived from the catalog — verified by: tests/catalog_sync.rs. Kind sets, class, tier and fields are checked as in round 1. Fields are now checked per kind. Mutation F (adding `foo` to the catalog's Paused/Resumed row) fails `payload_fields_match_the_catalog`.
- [x] Discriminants are stable, and a snapshot test fails if any value changes — verified by: `kind::tests::discriminants_are_frozen` (unchanged), plus the new `retired_discriminants_are_never_reused`. Mutation C (RETIRED = ("OldKind", 37)) fails it; control C2 (an unused 120) passes.
- [x] The codec round-trips all payloads under proptest — verified by: codec_roundtrip.rs is unchanged since round 1 and passes.
- [x] Every encoded payload is at most 12 bytes, and empty payloads encode to `None` — verified by: the same tests as round 1, unchanged and passing.
- [x] The decoder accepts every historical `payload_version` — verified by: payload_versions.rs is unchanged since round 1 and passes.

Round-1 blocking findings, re-verified by mutation on a scratch copy (git archive b13237e):
- Mutation A, swapping the QueueOrigin `Album`/`Playlist` codes, now fails `frozen::enum_codes_are_frozen_by_name` (frozen.rs:56). It passed all tests in round 1.
- Mutation B, catalog `` `mode` (`Off|All|Once`) ``, now fails `enum_values_listed_in_payload_columns_match_the_code` (catalog_sync.rs:443).
  - Control B2 (`Off|All|One`, correct) passes, so the list is really compared with the code and not just rejected.
  - Mutation B3 (a list on the unmapped `stars`) fails with the "map it in enum_names_for" panic (catalog_sync.rs:437).

Spot-checks on the new guards:
- **frozen.rs is not vacuous.** It compares live (name, value) lists built from `ALL`/`pack` against literal tables, in order.
  - Mutation J, swapping the `InputSource` Keyboard/MediaKey tags, fails `input_source_tags_are_frozen_by_name`.
  - Flag sets also check ALL_BITS against the frozen mask, so a flag added without a frozen row fails.
- **The RETIRED lists are empty today, but the logic works.**
  - D (QueueOrigin retired code 2) fails at frozen.rs:66.
  - E2 (MetadataFields retired bit 3) fails at frozen.rs:66.
  - E (an unknown enum name) fails `retired_entries_refer_to_known_enums`.
- **SPLIT_ROWS cannot hide drift on its own:**
  - Each split kind must equal its entry exactly. Mutation G (Paused claims `pause_ms`) fails at catalog_sync.rs:368.
  - Entries must be subsets of the row, and the row union must equal the catalog row (mutation F fails at :376).
  - A partially listed row fails. Mutation H (SessionStarted removed) fails at :332.
  - Stale entries fail.
  - Mutation I (adding a single-kind row `Rated`) passes, which is correct and harmless: for a single-kind row, the union check forces the kind to carry the full row anyway.
- **Header-driven columns work.** Mutation K inserts a `Bytes/day` column before `Tier` in the Volume table, and all tests pass. Rows must have one cell per header column.
- **Schema and encoding are unchanged since round 1.** `git diff dfa71af..b13237e` touches only enums.rs (test module removals), frozen.rs (new, behind `#[cfg(test)] mod frozen;` in lib.rs), kind.rs (test module only), payload.rs (`form_factor` doc comment only) and tests/catalog_sync.rs. codec.rs, quant.rs, event.rs, error.rs, codec_roundtrip.rs, payload_versions.rs, Cargo.toml, Cargo.lock and the crate manifest have an empty diff. `git diff main...HEAD` still touches no docs/ or work/ files.

blocking findings:
- none

design check: n/a

non-blocking findings:
- crates/tinwave-signals/src/frozen.rs:238-248 and crates/tinwave-signals/src/enums.rs:249-258 — the `InputSource` variant list in the frozen test and `InputSource::NAMES` are both hand-maintained. A new variant added only to the enum would get compiler-forced `name()`/`pack`/`unpack` arms, but its tag would be neither frozen nor compared with the catalog. Fix: derive the list from an exhaustive match (e.g. a helper whose match has no wildcard), so a new variant fails to compile until it is listed.
- crates/tinwave-signals/tests/catalog_sync.rs:306-311 — by design, editing `SPLIT_ROWS` and the code together can move a field off one sibling kind (e.g. drop `input` from `Paused`) without any catalog change. The table is explicit and visible in the diff, so this is acceptable. Reviewers of future changes to `SPLIT_ROWS` should treat it as a schema decision.

proposals:
- P: Exhaustive-match source for `InputSource::NAMES` and its frozen list | why: closes the last hand-typed list that a new variant could bypass | size: S

notes for lead:
- The branch is still 2 lead-only commits behind main (work/ and docs/rules/architecture.md); the branch touches neither, so no conflict is expected.
- The catalog-copy commit ec6479e is still a no-op against main.
- P-0011 enum lists will now be checked as soon as they land. `enum_names_for` already maps `mode`, `from_view`/`view`, `playlist_kind`, `scope`, `entity_kind`, `state` and RadioFeedback's `kind`. Any typo in those catalog lists will fail CI, which is intended.
- The mutation copies and scripts are in the session scratchpad (mut2/, run.sh, run2.sh). The repo and worktree were not modified.
