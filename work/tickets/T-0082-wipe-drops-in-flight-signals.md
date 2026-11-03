---
id: T-0082
title: A wipe also drops signal events and derived values still in flight
type: fix
epic: E-01
status: backlog
priority: P1
size: S
depends_on: [T-0070]
scope:
  - crates/tinwave-signals/src/
  - crates/tinwave-db/src/wipe.rs
  - crates/tinwave-db/src/writer.rs
  - crates/tinwave-db/tests/   # a new test file
  - docs/adr/0010-derived-values-on-the-bus.md   # Consequences paragraph
  - docs/rules/privacy-and-signals.md
owner:
branch:
review_round: 0
created_by: from P-0326 (T-0070 build-1)
---

## Context
After T-0070, a value or event queued on the bus or staged in the writer (at most one flush interval) when a
`SignalWiper` wipe commits is still stored afterwards: the wiper is a separate connection with no link to the bus or
the writer. Toggle-off is already re-checked at emit, drain and commit. A wipe must leave nothing of what it covers.

## Scope
- A wipe generation (or equivalent) shared by `SignalSettings`, the bus drain and the writer stage; the wiper bumps it
  inside its transaction, and anything emitted before the bump that the wipe covers is dropped, never stored.
- Same for events and derived values. Bounded: no lock on the emit path.

## Acceptance criteria
- [ ] An event and a derived value emitted just before a wipe of their category/tier are not in the database after the
      writer's next commit (tests, real writer thread, 20 iterations).
- [ ] Data emitted after the wipe is stored normally.
- [ ] Mutation (generation not compared) fails the tests.
- [ ] fmt, clippy, nextest, deny, `cargo build --workspace` pass.

## Signals & storage
No new signal. Privacy rule: a wipe covers in-flight data.
