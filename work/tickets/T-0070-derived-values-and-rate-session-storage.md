---
id: T-0070
title: Derived signal values as a first-class path; store RateSession with the play
type: feature
epic: E-01
status: ready
priority: P1
size: M
depends_on: [T-0062]
scope:
  - crates/tinwave-signals/src/
  - crates/tinwave-db/src/play.rs
  - crates/tinwave-db/src/store.rs
  - crates/tinwave-db/migrations/signals/   # a new migration only, if a column is needed
  - crates/tinwave-playback/src/controller/  # replacing take_rate_sessions with the new path
  - apps/tinwave-desktop/src/app/wiring.rs   # only if the composition root must connect them
  - docs/guides/signal-catalog.md            # RateSession row and §9 estimate
  - docs/rules/database.md, docs/rules/storage-budget.md   # play row columns only
owner:
branch:
review_round: 0
created_by: from P-0275, P-0276 (T-0062 build-1)
---

## Context
T-0062 derives one `RateSession{time_weighted_rate}` per heard play in the controller, but the bus refuses derived
kinds (`SignalsError::NotAnEvent`, StorageClass::Derived), so it is handed over through
`Controller/ControllerHandle::take_rate_sessions()` — a stopgap with no consumer. PerceivedLevel and later derived
values need the same path.

## Scope
- A derived-value path in tinwave-signals (e.g. a `DerivedValue` type the bus carries to the writer without storing it
  as a `signal_event` row), respecting the category toggles and the privacy tiers of its inputs.
- The writer stores RateSession with its play row (quantized as storage-budget.md says, e.g. a u8 rate column or the
  rate_up/rate_down flag bits plus a compact value), in the play-end transaction; wipes clear it with the Speed category
  (register it in signal_wipe.rs).
- Remove the `take_rate_sessions` stopgap once the path exists.
- An ADR if the derived-value path changes the signal architecture (ADR template, update the catalog/privacy docs).

- (P-0278) Consent is re-checked when a derived value is collected/stored; values queued before a Speed toggle-off
  or a wipe are dropped, not stored. Also give `Controller::take_rate_sessions` `&self`/`#[must_use]` if it survives.

## Acceptance criteria
- [ ] A heard play's RateSession ends up in its play row, quantized; none with Speed off — tests.
- [ ] A Speed wipe clears it (signal_wipe rule table + test).
- [ ] Storage estimate in catalog §9 and the growth test updated and passing.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
Stores an existing derived kind; ~1 byte per play (~0.1 KB/day).
