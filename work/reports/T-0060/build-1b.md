HANDOFF T-0060 (round 1, updated for the lead's rulings)
status: done (only the context_snapshot.fields blob item remains open, accepted by the lead as out of scope)
branch: t/T-0060-wipe-masks-context-key   commit: 209413c (on top of 7d581e6)
summary: The lead accepted that no wipe existed. I built `SignalWiper` in tinwave-db: its own connection, an IMMEDIATE transaction, and it rewrites `play.context_key` and `context_snapshot.key`. After the ruling, each ContextKey field has exactly one owning category and may also be derived from other categories, and a wipe of any category clears what it owns plus what is derived from it. Activity is owned by ForegroundApp and derived from NetworkPlace and Clock, so wiping any of the three clears it. Rollups are untouched, and turning a category off changes no rows.

acceptance criteria:
- [x] Wiping each T2/T3 context category zeroes exactly its ContextKey fields in every play row; other fields untouched. Proof: `wiping_a_category_clears_exactly_its_fields_in_every_play_row` covers all 5 categories; its expectations come only from the typed `with_*` builders and now include derived fields. It also checks that the other `play` columns are unchanged. `wiping_a_tier_clears_the_fields_of_its_categories` covers T0 to T3: T1 now clears daypart, weekday, device and activity; T2 clears activity and place; T3 clears condition and temperature. Also `several_categories_at_once_and_a_second_wipe_changes_nothing`, `many_rows_with_the_same_key_are_all_rewritten`, and the end-to-end `a_real_play_loses_the_wiped_fields_and_events_and_rollups_stay` (events and rollups unchanged).
- [x] Derived fields (lead ruling): `wiping_the_place_or_the_clock_also_clears_the_derived_activity` shows that wiping NetworkPlace, Clock or ForegroundApp clears activity and leaves the device class. It also shows that OutputDevice and Weather leave activity alone. The unit test `activity_is_cleared_by_the_categories_it_is_derived_from` pins the same rule. `every_key_field_has_exactly_one_owning_category` now checks `owned_fields` for exactly one owner per field and that no field is both owned and derived for the same category.
- [x] A key whose last known field is wiped becomes raw 0. Proof: `a_key_whose_last_field_is_wiped_becomes_raw_zero` (db) and `clearing_the_last_known_field_gives_raw_zero` (signals).
- [x] The wipe is atomic. Proof: `a_failure_after_the_play_rows_were_rewritten_changes_nothing` (a trigger aborts the snapshot UPDATE after the plays were rewritten; plays and snapshots are unchanged, and the wipe succeeds once the fault is removed). Also `a_failure_in_the_middle_of_the_play_rewrite_changes_nothing` and `a_stored_key_that_is_not_a_u32_fails_the_wipe_and_changes_nothing`.
- [x] Turning a category off stops new values only. Proof: `turning_a_category_off_leaves_existing_rows_alone`.
- [x] fmt, clippy, nextest, deny pass; the storage growth tests still pass. Proof: fmt exit 0, clippy `-D warnings` exit 0, nextest "663 tests run: 663 passed (4 slow), 6 skipped", `cargo deny check` reports "advisories ok, bans ok, licenses ok, sources ok".
- [ ] `context_snapshot.fields` blob wipe — why not: the blob has no encoding yet. The lead accepted this as out of scope, and the ticket that defines the encoding extends `SignalWiper`. It is documented in the wipe.rs module docs under "Not yet covered".

Revert test: I temporarily removed the derived-fields entry for Clock and NetworkPlace in `derived_fields`. 7 tests failed, including the new ones. I also tested the earlier mutations: dropping the raw-0 normalization in `clear_raw` failed 7 tests, and committing between the play and snapshot rewrites failed `a_failure_after_the_play_rows_were_rewritten_changes_nothing`. The files are restored and the tree is clean.

checks: fmt pass, clippy pass, tests 663 passed / 0 failed, deny pass
files changed:
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-signals\src\context_key.rs (`ContextField`, `ContextFieldSet`, `ContextKey::without`, `ContextKey::clear_raw`, tests)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-signals\src\lib.rs (re-export; accepted outside the literal scope)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\src\wipe.rs (new: `ContextCategory` with `owned_fields`, `derived_fields` and `fields`; `SignalWiper`; `WipeReport`; the owned and derived rule is in the module docs)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\src\lib.rs (mod, re-exports, crate docs)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\tests\context_wipe.rs (new, 11 tests)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\docs\rules\privacy-and-signals.md (Wipe bullet: one owner per field plus the derived rule)
signals added/changed: none (rows are updated in place; no storage change)
ui: none
docs updated: docs/rules/privacy-and-signals.md (Wipe bullet); module docs in wipe.rs and context_key.rs

Ownership and derivation table (as implemented):
| category | tier | owns | also clears (derived from it) |
|---|---|---|---|
| Clock | T1 | daypart, weekday type | activity |
| OutputDevice | T1 | device class | none |
| ForegroundApp | T2 | activity | none |
| NetworkPlace | T2 | place slot | activity |
| Weather | T3 | weather condition, temperature band | none |
- Idle, fullscreen and power also feed the activity guess, but they are not modeled categories, so they clear nothing.
- Calendar (T2) and T0 have no ContextKey field.

deviations from ticket:
- Built the wipe (`SignalWiper`, `ContextCategory`) because none existed; the lead accepted it.
- Edited `crates/tinwave-signals/src/lib.rs` (the re-export) outside the literal scope; accepted.
- `clear_raw` works on bits, so a wipe also cleans a key this build cannot decode. A stored key that is not a `u32` fails the wipe with `DbError::OutOfRange` and rolls back instead of leaving data behind.

proposals:
- P: Wire the Signal Center wipe action to `SignalWiper` | why: nothing calls it yet; the UI ticket needs the context categories and the tier wipe | size: S | evidence: crates/tinwave-db/src/wipe.rs
- P: Wipe `context_snapshot.fields` in the ticket that defines the blob encoding | why: the blob will hold weather, place and app values that this wipe cannot touch yet | size: S | evidence: crates/tinwave-db/migrations/signals/0001_init.sql:44
- P: Wipe-everything and per-signal-category wipe (delete `signal_event` rows, clear `taste.db`) | why: privacy-and-signals.md promises "per category, per tier, everything" and this ticket covers context fields only | size: M | evidence: docs/rules/privacy-and-signals.md

questions for lead: none

Process note: nothing I started is still running. A final check found no tinwave-desktop or image-stress. Earlier `cargo`, `cargo-nextest` and `python` processes belonged to other agents and I left them alone.
