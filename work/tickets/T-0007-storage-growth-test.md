---
id: T-0007
title: Storage growth test v1 — 5 simulated years of a heavy listener
type: feature
epic: E-00
status: backlog
priority: P1
size: M
depends_on: [T-0006]
scope:
  - crates/tinwave-signals/benches/
  - crates/tinwave-signals/src/sim/
  - crates/tinwave-db/tests/
owner:
branch:
review_round: 0
created_by: lead
---

## Context
docs/rules/testing.md "Storage growth test", docs/rules/storage-budget.md, signal catalog §9 volume estimates.

## Scope
- Seeded synthetic listener generator: 8 h/day, ~120 tracks/day, 50 000-track library, catalog-rate volume,
  seek, skip, queue, search and UI counter activity; context changes ~60/day.
- Run v1 without compaction (compaction is an E-01 ticket) for 90 simulated days and extrapolate; structure
  the harness so the 5-year run with compaction plugs in later.
- Report per-table sizes and bytes/day per signal group vs the catalog table.

## Acceptance criteria
- [ ] Deterministic with a fixed seed — same sizes across runs.
- [ ] 90-day hot window ≤ 6 MB — assertion.
- [ ] Per-group bytes/day within +50 % of the catalog estimates — assertion with a readable report.
- [ ] Runs in < 60 s in release mode.
- [ ] Volume-gesture bursts are in the simulation: a 60-step system-volume burst within 200 ms stores exactly one
      `SystemVolumeChanged` (P-0010, ADR 0004) — assertion.

## Signals & storage
Validates ADR 0002 early. If estimates are wrong, propose catalog/budget updates rather than loosening asserts.
