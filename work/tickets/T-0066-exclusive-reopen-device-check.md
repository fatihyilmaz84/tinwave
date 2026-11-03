---
id: T-0066
title: Exclusive reopen on a real device — ignored measurement test, exclusive-only open, harness settle
type: chore
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0057]
scope:
  - crates/tinwave-playback/src/output/windows.rs
  - crates/tinwave-playback/src/output/mod.rs     # OpenRequest flag only
  - crates/tinwave-playback/src/engine.rs         # use of the flag only
  - crates/tinwave-playback/tests/device.rs
  - crates/tinwave-playback/tests/common/mod.rs
  - spikes/T-0003-audio-backend/PO-RUNBOOK.md     # one new section
owner:
branch:
review_round: 0
created_by: from P-0263, P-0264, P-0266 (T-0057 build-1)
---

## Context
T-0057 implemented the exclusive-mode reopen at the file rate on fakes only. The real gap (device buffer + exclusive
open + first block + poll) and a real endpoint's refusal are unmeasured, and a refusal costs a wasted shared open.

## Scope
- An `#[ignore]` device test (silent-clip pattern from tests/device.rs) that plays a 44.1 kHz then 96 kHz silent pair in
  exclusive mode and prints the reopen time and the mode/rate actually opened; a PO-RUNBOOK section to run it and to
  listen for the gap on a real DAC with a real album.
- `OpenRequest` "exclusive only" flag: WasapiOutput fails instead of falling back to shared, so the engine retries at its
  own rate in one open.
- `Harness::settle` polls like `settle_polling` (no spin in exclusive mode).

## Acceptance criteria
- [ ] Ignored test compiles and is excluded from the default run (default run opens no device).
- [ ] A refused rate costs one failed open + one open at the engine rate (ManualOutput counts opens) — test.
- [ ] settle() works in exclusive mode — test.
- [ ] fmt, clippy, nextest, deny pass.

## Signals & storage
None.
