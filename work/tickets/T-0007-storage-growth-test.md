---
id: T-0007
title: Storage growth test v1 — 5 simulated years of a heavy listener
type: feature
epic: E-00
status: in_progress
priority: P1
size: M
depends_on: []          # T-0006 done
scope:
  - crates/tinwave-db/tests/              # reuse/extend the T-0006 realistic Listener in tests/common
  - crates/tinwave-db/benches/
  - crates/tinwave-db/Cargo.toml          # dev-dependencies only
  - Cargo.toml                            # [workspace.dependencies] only
  - Cargo.lock
owner: tw-builder (round 1, same builder as T-0006)
branch: t/T-0007-storage-growth-test
review_round: 0
created_by: lead
---

## Context
Lead grooming 2026-11-02: the generator lives in tinwave-db test code (where T-0006 already built a realistic
`Listener`), not in a production crate. T-0006 measured 39.4 B/event (table) and 48.2 B/event (whole file).
Compaction and play records (T-0029) are not built yet: v1 measures the hot tier and extrapolates.
docs/rules/testing.md "Storage growth test", docs/rules/storage-budget.md, signal catalog §9 volume estimates.

## Scope
- Seeded synthetic listener generator: 8 h/day, ~120 tracks/day, 50 000-track library, catalog-rate volume,
  seek, skip, queue, search and UI counter activity; context changes ~60/day.
- Run v1 without compaction (compaction is an E-01 ticket) for 90 simulated days and extrapolate; structure
  the harness so the 5-year run with compaction plugs in later.
- Report per-table sizes and bytes/day per signal group vs the catalog table.
- Generate events as real `tinwave-signals` `SignalPayload`s and encode them with the T-0004 codec, so sizes are
  measured, not estimated (P-0013).

## Acceptance criteria
- [ ] Deterministic with a fixed seed — same sizes across runs.
- [ ] 90-day hot window ≤ 6 MB — assertion.
- [ ] Per-group bytes/day within +50 % of the catalog estimates — assertion with a readable report.
- [ ] Runs in < 60 s in release mode.
- [ ] Volume-gesture bursts are in the simulation: a 60-step system-volume burst within 200 ms stores exactly one
      `SystemVolumeChanged` (P-0010, ADR 0004) — assertion.

## Signals & storage
Validates ADR 0002 early. If estimates are wrong, propose catalog/budget updates rather than loosening asserts.
