---
id: T-0023
title: Live queue edits in the playback engine (play next, reorder, remove) without a flush
type: feature
epic: E-01
status: in_review
priority: P1
size: M
depends_on: [T-0016]
scope:
  - crates/tinwave-playback/
owner: tw-builder
branch: t/T-0023-live-queue-edits
review_round: 2
created_by: from P-0041
---

## Context
T-0016's engine has `load` (flushes) and `append` only. Queue edits during playback (spec §7; catalog §4
`QueuedNext/Later`) must not interrupt the playing track or break gapless hand-off.

## Scope
- Decoder-side replace of the items after the head, with a stale-head check (the edit applies only if the head is
  still the item the UI saw).

## Acceptance criteria
- [ ] Play-next / reorder / remove while playing never interrupts the current track — test.
- [ ] Gapless hand-off into an edited next item stays 0-frame — test.
- [ ] A stale edit (head changed) is rejected with a visible error, not applied to the wrong item — test.
- [ ] Zero callback allocations still hold (rt_alloc test).
- [ ] Each edit kind emits its catalog signal once, in the open session (or opens none if no session: queue edits
      outside a session are recorded only if the catalog says so — check §4/§8 and state the rule used) — tests.

## Signals & storage
Queue edits express preference, so they emit signals in this ticket (CLAUDE.md hard rule), through the T-0024
controller: play next → `QueuedNext`, add to queue → `QueuedLater` (payload `from_view`, passed by the caller),
reorder → `QueueReordered` (`distance`, signed), remove before it played → `RemovedFromQueue` (`was_radio_pick`), each
with the caller's `InputSource`, per catalog §4. A rejected (stale) edit emits nothing. One emission test per kind;
a disabled category constructs nothing. No new kinds; if a payload field the catalog names cannot be filled here, say
so in the handoff instead of inventing a value.

## Added from P-0112 (T-0024 review)
Next/Previous commands carry the queue item they expect to leave; if the engine already auto-advanced past it, the command is a no-op (no reload, no < 5 s Skipped on the track the user wanted). Test at a track boundary.

## Added from T-0034 build-2 (P-0143)
- Test helper `Rig::render_until_heard` in tests/controller_support: wait for the pending command before counting
  frames (the stale-frame hazard T-0034 round 2 root-caused); use it instead of fixed-count `hear()` where it matters.
- Random-walk property (P-0147): no `SessionEnded.minutes` exceeds the audible time between its `SessionStarted`
  and its last audible event.
