HANDOFF T-0060 (round 1)
status: partial
branch: t/T-0060-wipe-masks-context-key   commit: 7d581e6
summary: The ticket says "extend the existing category/tier wipe", but no wipe exists anywhere in the repo. I found no wipe, delete or clear code in crates/ (only docs mention it) and the Signal Center is not built yet. So I built the smallest wipe that satisfies the ticket's acceptance criteria. It is `SignalWiper` in `tinwave-db`, which clears a context category's or tier's fields from `play.context_key` and `context_snapshot.key` in one transaction. Plays and snapshot keys are done and tested. The `context_snapshot.fields` blob cannot be wiped yet because it has no encoding (see questions), so I mark this partial.

acceptance criteria:
- [x] Wiping each T2/T3 context category zeroes exactly its ContextKey fields in every play row; other fields untouched — proof: `wiping_a_category_clears_exactly_its_fields_in_every_play_row` runs over all 5 categories (T1, T2, T3), with expectations built only from the typed `with_*` builders. It also checks that the other `play` columns are unchanged. `wiping_a_tier_clears_the_fields_of_its_categories` covers T0 to T3. `several_categories_at_once_and_a_second_wipe_changes_nothing`, `many_rows_with_the_same_key_are_all_rewritten` and the end-to-end `a_real_play_loses_the_wiped_fields_and_events_and_rollups_stay` also pass. The end-to-end test goes write_batch, snapshot key, play row, wipe, and checks that raw events and `rollup_monthly` are untouched.
- [x] A key whose last known field is wiped becomes raw 0 — proof: `a_key_whose_last_field_is_wiped_becomes_raw_zero` (db) and `clearing_the_last_known_field_gives_raw_zero` (signals).
- [x] The wipe is atomic (a failure leaves play rows and hot-tier data unchanged) — proof: `a_failure_after_the_play_rows_were_rewritten_changes_nothing` uses a trigger that aborts the snapshot UPDATE after the play rows were rewritten. Both play rows and snapshots are unchanged afterwards, and the same wipe succeeds once the trigger is dropped. `a_failure_in_the_middle_of_the_play_rewrite_changes_nothing` and `a_stored_key_that_is_not_a_u32_fails_the_wipe_and_changes_nothing` also pass. "Hot-tier data" here means `context_snapshot.key` only (see questions).
- [x] Turning a category off stops new values only — proof: `turning_a_category_off_leaves_existing_rows_alone` toggles tiers and categories through `PersistentSettings`, and the rows stay the same.
- [x] fmt, clippy, nextest, deny pass; storage growth test still passes — proof: `cargo fmt --all --check` exit 0; `cargo clippy --workspace --all-targets -- -D warnings` exit 0; nextest Summary "661 tests run: 661 passed, 6 skipped" (includes the storage_growth tests); `cargo deny check` reports "advisories ok, bans ok, licenses ok, sources ok".
- [ ] "Existing wipe extended" — why not: there was no wipe to extend, so I created one.
- [ ] The wipe also covers `context_snapshot.fields` — why not: that blob has no encoding yet (see questions).

checks: fmt pass, clippy pass, tests 661 passed / 0 failed, deny pass
files changed:
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-signals\src\context_key.rs (`ContextField`, `ContextFieldSet`, `ContextKey::without`, `ContextKey::clear_raw`, module doc paragraph, tests)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-signals\src\lib.rs (re-export of the two new types; one line outside the literal scope, see deviations)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\src\wipe.rs (new: `ContextCategory`, `SignalWiper`, `WipeReport`)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\src\lib.rs (mod and re-exports, crate docs)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\crates\tinwave-db\tests\context_wipe.rs (new, 10 tests)
- D:\dev\Tinwave-worktrees\T-0060-wipe-masks-context-key\docs\rules\privacy-and-signals.md (the Wipe bullet only)
signals added/changed: none (rows are updated in place; no new signal, no storage change)
ui: none
docs updated: docs/rules/privacy-and-signals.md (wipe paragraph); wipe.rs and context_key.rs module docs

Field-to-category mapping (the owning category is ambiguous for activity; I made the call and it needs your confirmation):
| category | tier | ContextKey fields |
|---|---|---|
| Clock | T1 | daypart, weekday type |
| OutputDevice | T1 | device class |
| ForegroundApp | T2 | activity |
| NetworkPlace | T2 | place slot |
| Weather | T3 | weather condition, temperature band |
- Calendar (T2) and T0 own no key field.
- The categories are provider groups from context-engine.md with the tiers of signal-catalog §7. `SignalCategory` has no context categories, so I defined `ContextCategory` in `tinwave-db`.
- Activity is the ambiguous one. It is derived from app category (T2), idle and fullscreen (T1), power plus place, and clock. I assigned it to ForegroundApp (T2) because its sensitive values (Focus, Gaming, Call) come from there. A T1 or place wipe therefore leaves the activity guess in place. That means a place wipe leaves an "OnTheMove" guess that was derived partly from place. Say so if you want activity cleared by more categories. It is a one-line change in `ContextCategory::fields`, and the test `every_key_field_has_exactly_one_owning_category` would then need adjusting.

deviations from ticket:
- Built the wipe itself (`SignalWiper`, `ContextCategory`) because none exists. It is an independent connection to `signals.db`, like `PersistentSettings`, and uses an IMMEDIATE transaction. It does not go through `SignalStore`, because the writer thread owns the store.
- Edited `crates/tinwave-signals/src/lib.rs` (the re-export line) so `tinwave-db` can use the new field types. Scope listed only `context_key.rs`.
- `clear_raw` works on bits and does not decode, so a wipe also cleans a key this build cannot decode. A stored key that is not a `u32` fails the wipe with `DbError::OutOfRange` and rolls back, so a wipe never reports success with data left behind.

proposals:
- P: Wire the Signal Center "wipe" action to `SignalWiper` | why: nothing calls it yet; the UI ticket has to expose the context categories and the tier wipe | size: S | evidence: crates/tinwave-db/src/wipe.rs
- P: Make the `context_snapshot.fields` blob wipe part of whichever ticket defines the blob encoding | why: the blob will hold weather, place and app values that this wipe cannot touch until then (wipe.rs module docs "Not yet covered") | size: S | evidence: crates/tinwave-db/migrations/signals/0001_init.sql:44
- P: A wipe-everything / per-signal-category wipe (delete `signal_event` rows, clear `taste.db`) is still missing | why: privacy-and-signals.md promises "per category, per tier, everything" and this ticket only covers context fields | size: M | evidence: docs/rules/privacy-and-signals.md

questions for lead:
- Is it acceptable that `context_snapshot.fields` is not wiped yet? Today nothing writes snapshots, so no data is exposed. I did not want to invent a blob format or blank the whole blob. Should the blob wipe wait for the ticket that defines the encoding?
- Do you confirm the activity-to-ForegroundApp mapping?

Process note: when I finished, `cargo.exe`, `cargo-nextest.exe` and `python.exe` were still running. I did not start them (all my commands completed), so I left them alone; they are probably the T-0027 or T-0058 agents.
