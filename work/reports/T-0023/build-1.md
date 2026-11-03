HANDOFF T-0023 (round 1)
status: done
branch: t/T-0023-live-queue-edits   commit: b9eb089
summary: `Engine::replace_after(head, items)` replaces everything after the last item the decoder started, with no flush or reload; the current track keeps playing. The hand-off into an edited next item is bit-exact against the uncut sweep. `Controller::edit_queue` and `queue_later` (also on `ControllerHandle`) emit QueuedNext, QueuedLater, QueueReordered and RemovedFromQueue once per item, only once the decoder has reported the edit applied. P-0112, P-0143 and P-0147 are in. Size M.

Design:
- The decoder reads up to ~1 s ahead, so in a track's last second the next item is already in the ring and can't be edited; the decoder thread has the last word.
- Staleness detected in two places: control side compares `head` with the current item (mismatch → `PlaybackError::StaleQueue`); decoder side refuses the read-ahead race with `EventKind::QueueEditRejected` after the call succeeded.
- Signals are emitted on `QueueEdited` (applied), never before. A rejected edit emits nothing, is logged at warn and counted in `ControllerDiagnostics::edits_refused`; `edits_pending` counts edits awaiting a verdict.
- `queue_later` has no verdict (append can't be refused), so it records at once.
- New public API: `Engine::replace_after`, `EditId`, `EventKind::{QueueEdited, QueueEditRejected}`, `PlaybackError::StaleQueue`, `QueueEdit`, `QueueEditRequest`, `LoadRequest.leaving`, `ControllerDiagnostics::{edits_pending, edits_refused}`.

acceptance criteria:
- [x] Play-next / reorder / remove never interrupts the current track — tests/queue_edits.rs: `an_edit_during_playback_neither_interrupts_nor_gaps_the_hand_off` (A's audio bit-exact, `command_pending` false, no TrackLeft{Replaced|Stopped|Failed}); `reordering_the_tail_plays_the_new_order`; `removing_from_the_tail_skips_it_and_removing_all_ends_after_the_current_track`.
- [x] Gapless hand-off into an edited next item is 0-frame — the first test records A then X vs the uncut sweep, `max_err == 0.0`, `best_lag` 0 at the cut. Also `an_edit_after_the_decoder_found_the_end_of_the_queue_continues_it` (edit on the last item revives the stream gaplessly; B left at 4800 frames).
- [x] A stale edit is rejected with a visible error and applies nothing — control side: `an_edit_built_on_a_track_that_is_no_longer_current_is_refused_by_the_call` (StaleQueue), `an_edit_with_nothing_playing_or_with_duplicate_ids_is_an_error_not_a_change`; decoder side: `an_edit_that_loses_the_race_with_the_decoder_is_refused_with_an_event`, `an_edit_that_repeats_an_id_of_an_earlier_item_is_refused_by_the_decoder`; controller: `an_edit_built_on_a_track_that_has_ended_emits_nothing`, `an_edit_the_decoder_refuses_emits_nothing_and_is_counted`.
- [x] Zero callback allocations — `rt_alloc` and `controller_rt_alloc` now include replace_after, play next, queue later, reorder and remove while the callback runs; green. `output_rt_alloc` unchanged, green.
- [x] Each edit kind emits its catalog signal once, in the open session — tests/controller_edits.rs: `play_next_emits_one_queued_next_per_inserted_item_in_the_open_session`, `queue_later_emits_one_queued_later_per_added_entry`, `reordering_emits_queue_reordered_with_the_signed_distance`, `removing_emits_removed_from_queue_and_knows_a_radio_pick`, `a_disabled_category_constructs_no_queue_signal` (bus `queued_total` unchanged; next signal gets seq 2).
- [x] P-0112: `next_pressed_on_a_track_that_has_already_ended_does_nothing` at a real gapless boundary (A ended, B heard, display still shows A): Next and Previous change nothing (no reload, no Skipped, B keeps playing); Next with `leaving` = the real current item works as before; `choosing_a_track_does_not_depend_on_what_was_playing` (Select ignores `leaving`).
- [x] P-0143: `Rig::render_until_heard` (settles first, renders until a new PlayStarted; timeout panic prints status and diagnostics), used in the new tests.
- [x] P-0147: the random walk checks no SessionEnded.minutes exceeds the audible time (samples `status()` once per step, 1 min slack); default seeds 24 → 48. Mutation: removing `session.set_audible(at, !self.paused)` in `ensure_session` fails at seed 42. The walk also includes the four edit ops and asserts queue signals never exceed accepted edits, and at least one was recorded.

Session rule used (no session open):
- An edit with a listener's input on a not-yet-heard play (restored paused at launch) opens the session with the play's origin, via `open_session_for_unheard`.
- Otherwise (nothing loaded, or the play outlived its session) the edit is applied and not recorded, like a seek or volume change; not deferred like repeat mode.
- `InputSource::Auto` (the queue layer's own refill) applies and records nothing.
- The four kinds' catalog payloads have no `input` field; input only separates a listener's act from `Auto`.

Payload fields: `from_view` and `distance` passed by the caller; `was_radio_pick` from the entry's `QueueOrigin::Radio`; signals scoped to each item's recording and source; `pos_ds` none. `Remove` of an item the mapper never saw returns `UnknownItem`.

checks: fmt pass, clippy pass, tests 503 passed / 0 failed / 6 skipped in 5 full `cargo nextest run --workspace` runs, deny pass. `RUSTDOCFLAGS=-D warnings cargo doc -p tinwave-playback` passes. Stress: `--stress-count 30 -j 24` over queue_edits, controller_edits, controller_thread, controller_random and pairing, 30/30. No storage_growth timeout.

Mutation checks (each killed, then restored): decoder ignores the head check → decoder-race tests fail; control side skips the stale check → stale tests fail; Next/Previous ignore `leaving` → boundary test fails; Auto edits recorded → Auto test fails; decoder doesn't revive the stream after an edit on the ended queue → last-item test fails in 2 of 3 runs (it sleeps 200 ms so the decoder reaches the end first; a slower decoder takes the other correct path).

files changed: crates/tinwave-playback/src/{decoder,engine,error,event,lib,types}.rs; src/controller/{mapper,mod,runtime,types}.rs; tests/{queue_edits,controller_edits}.rs (new); tests/{controller_random,controller_rt_alloc,controller_signals,controller_support/mod,controller_thread,pairing,rt_alloc}.rs; docs/guides/signal-catalog.md

signals added/changed: no new kinds. Emission added for QueuedNext, QueuedLater, QueueReordered, RemovedFromQueue (T0). Volume already inside the catalog's queue group (~40 events/day, ~2.5 KB/day); an album added at once is ~15 events at ~39 B. No estimate or growth-test change. Signal Center list unchanged (kinds exist).
ui: none
docs updated: signal-catalog.md §8 "Session boundaries" rule for queue edits and the `RemovedFromQueue` trigger text in §4; controller mod.rs "Queue edits" section, mapping table and session bullets; docs on `Engine::replace_after` and the new events/types.

deviations from ticket:
- signal-catalog.md edited (outside scope) because the lead asked for §8 in the same change.
- Public API shape: `edit_queue` takes the full new tail plus a `QueueEdit` description (PlayNext, Reorder, Remove); `LoadRequest.leaving` is a new field (four test files changed). The shell/queue layer owns order and old tail; the controller validates the description against the tail (`InvalidArgument`/`UnknownItem`) but can't check a reorder's `distance`.

proposals:
- P: Deferred queue acts for the no-session case (bounded buffer reported after the next SessionStarted, like repeat mode) | why: edits made with nothing loaded or after the session expired are applied but never recorded | size: S | evidence: mapper.rs `record_queue_acts`, catalog §8
- P: Surface refused edits to the shell (fold into P-0135, Diagnostics wiring) | why: a decoder-refused edit succeeds at the call and shows only as `edits_refused` | size: S
- P: Engine-level queue mirror as a design point of the E-01 queue view-model ticket | why: PlayNext/Reorder/Remove trust the caller's description | size: S

questions for lead:
- Is "no session and no unheard play ⇒ applied but not recorded" acceptable?
- Last item: an edit reaching the decoder after the output played the last frame finds the queue ended; what plays afterwards is announced when heard. Documented on `replace_after` rather than fixed (a fix would need the renderer to wait on the decoder).

---
Lead (2026-11-03), sent back before review (same round):
- Session rule: not acceptable as is. CLAUDE.md: every interaction that expresses preference MUST emit a signal, and
  "add to queue" with nothing loaded is the common first act at launch. Lead decision → P-0160 accepted into this
  ticket: listener (non-Auto) queue acts with no session and no unheard play are kept in a bounded buffer (cap 64
  items; on overflow the oldest is dropped and counted in `ControllerDiagnostics`) and reported right after the next
  `SessionStarted`, stamped like a deferred RepeatModeChanged (session-start time). A buffered RemovedFromQueue for
  an item later re-added, or acts on items whose recording is unknown, follow the same rules as live ones. Catalog §8
  states it.
- Last-item edge: accepted as documented.
- P-0161 (surface refused edits) merged → P-0135 (parked, Diagnostics wiring). P-0162 (queue mirror) parked → E-01
  queue view-model ticket.
- catalog edit outside scope: accepted (the lead asked for it).
