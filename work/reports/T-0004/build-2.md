HANDOFF T-0004 (round 2)
status: done
branch: t/T-0004-signal-schema-codec   commit: b13237e
summary: All six review items are done. The only non-test changes are doc comments and the `mod frozen;` registration; schema and encoding are untouched. I could not read the review file: D:\dev\Tinwave\work\reports\T-0004\review-1.md does not exist, and only build-1.md is there. P-0020 and P-0021 are empty (front matter only). I worked from your message, so please check I did not miss a finding that was only in the file.

acceptance of the round-2 items:
- [x] 1. Names frozen with their values, per persisted enum.
  - New test-only module `src/frozen.rs` freezes `(name, code)` pairs for all 12 coded enums, the `InputSource` tag bytes (Mouse 0, Keyboard 1, MediaKey 2, Tray 3, MiniPlayer 4, Auto 5, Remote 6 plus the slot byte, System 7), and the flag bit positions by name for `MetadataFields` and `ContextScope`.
  - The old value-list-only test is removed from enums.rs.
  - Mutations, each restored afterwards:
    - Swapping `QueueOrigin::Album` and `Playlist` fails `frozen::enum_codes_are_frozen_by_name`.
    - Swapping the `TITLE` and `ARTIST` bits fails `frozen::flag_bit_positions_are_frozen_by_name`.
    - Swapping the `Keyboard` and `MediaKey` tags fails `frozen::input_source_tags_are_frozen_by_name`.
- [x] 2. Catalog value lists for unknown fields now fail.
  - `enum_names_for` pre-maps: `queue_origin`, `origin`/`start_origin`, `class`/`endpoint_class`, `transport`, `mode`, `from_view`/`view`, `playlist_kind`, `scope`, `entity_kind`, `state`, and `kind` (RadioFeedback only).
  - `input` is the only field handled elsewhere, in `common_enums_match_the_code`. Any other field with a value list panics with "map it in enum_names_for".
  - The catalog's current lists (`queue_origin`, `origin`, `class`, `transport`) must be found, so a broken parser can't pass silently.
  - Mutations, each restored afterwards: `mode (Off|All|Once)` fails. `stars (1|2)` fails with the "does not know" message.
- [x] 3. Columns are found by header name (`Signal`, `Tier`, `Payload`) through a new `Columns` struct, and each row must have one cell per header column. Mutation: I inserted a `bytes/day` column before `Tier` in the Volume table; the test still passed (restored afterwards).
- [x] 4. Multi-kind rows are now checked per kind.
  - The catalog gives one payload cell per row, with no per-kind lists.
  - Rows without a split (`QueuedNext / QueuedLater`, `PlaylistAdded / PlaylistRemoved`, `Favorited / Unfavorited`): each kind must carry exactly all the row's fields.
  - Four kinds are in a documented `SPLIT_ROWS` table, my Q7/Q1 reading of the shared rows: `Paused`={input}, `Resumed`={pause_ms,input}, `SessionStarted`={start_origin}, `SessionEnded`={tracks,minutes}. Each must equal its entry, the entries must be subsets of the row, and together they must cover the row.
  - A row must be entirely in the table or entirely out of it, and a stale entry fails the test.
  - Mutation: replacing `Paused`'s `input` field with another field fails ("Paused: packed payload fields differ from the catalog row"). Restored afterwards.
- [x] 5. The `form_factor` doc now says it is the raw OS endpoint form-factor value (Windows `EndpointFormFactor`: 0 = `RemoteNetworkDevice`, `UnknownFormFactor` = 10) and that 0 does not mean unknown.
- [x] 6. `RETIRED` lists exist and are checked, all empty now. In `kind.rs` the new test `retired_discriminants_are_never_reused` covers the discriminants. In `frozen.rs` the tests cover enum codes, `InputSource` tags and flag bit positions, and check that `RETIRED` only names known enums. The instructions to move a deleted variant's row into `RETIRED` are in the doc comments.
- [x] Optional: `TYPE_WORDS` now includes `bool`, `i8`..`i64`, `u8`..`u64`.

checks: fmt pass, clippy pass (workspace, all targets, -D warnings), tests 93 passed / 0 failed (39 core + 33 signals unit + 8 catalog_sync + 7 codec_roundtrip + 6 payload_versions), deny pass (advisories, bans, licenses, sources ok)
files changed: crates/tinwave-signals/src/{frozen.rs (new), enums.rs, kind.rs, lib.rs, payload.rs}, crates/tinwave-signals/tests/catalog_sync.rs
signals added/changed: none
docs updated: none (doc comments only)
deviations from ticket: none
proposals: none
questions for lead: none

---
Lead note: review-1.md was written right after the round-2 message (a shell quoting failure delayed it); the message
carried every finding, so nothing was missed. P-0020/P-0021 are short by design (outcome lines).
