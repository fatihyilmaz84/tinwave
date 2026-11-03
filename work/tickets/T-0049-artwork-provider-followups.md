---
id: T-0049
title: Artwork provider follow-ups — one max-edge source, Image.Error for refused/missing, one-edge sourceSize
type: feature
epic: E-01
status: in_progress
priority: P1
size: S
depends_on: [T-0044]
scope:
  - apps/tinwave-desktop/cpp/
  - apps/tinwave-desktop/src/app/artwork_bridge.rs
  - apps/tinwave-desktop/src/image/
  - apps/tinwave-desktop/tests/image_provider.rs
  - apps/tinwave-desktop/tools/stress/      # a fixture scene for the size cases, if needed
owner: tw-builder
branch: t/T-0049-artwork-provider-followups
review_round: 1
created_by: from P-0150, P-0151, T-0044 review-2, P-0169
---

## Context
T-0044 review-2 (work/reports/T-0044/review-2.md): `kMaxEdge = 1024` in cpp/image_provider.cpp duplicates
`MAX_THUMBNAIL_EDGE` in provider_core.rs; refused, missing and cancelled responses finish as `Image.Ready` at 0x0
(no `errorString()`); a one-dimension sourceSize (64x0) becomes a 128 square instead of keeping the aspect ratio.

## Scope
- The C++ limit comes from Rust (passed through `tw_provider_handle_new`, or a shared constant checked at build time).
- Refused and no-artwork responses report an error string, so QML gets `Image.Error`; a cancelled response stays
  silent (QML has moved on).
- One-edge sourceSize keeps the given edge and derives the other from the source aspect (or square when unknown),
  documented.
- Keep every ADR 0008 lifetime rule; re-run the stress target (>= 300 Release runs).

## Acceptance criteria
- [ ] Test: oversize request → `Image.Error`, no task run; no-artwork key → `Image.Error`; in-range → `Image.Ready`.
- [ ] Test: changing the Rust limit changes the C++ refusal (no second literal).
- [ ] Test: 64x0 and 0x64 requests produce a 64-edge image.
- [ ] Stress >= 300 Release runs, 0 crashes, 0 hangs, `duplicate_finishes=0`.
- [ ] fmt, clippy -D warnings, tests, deny pass.

## Signals & storage
None.

## Added from P-0169 (T-0043 review-2) — gate flake, P1
- `tests/image_provider.rs::cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` failed 2 of 14 runs under
  full-workspace load ("no run quit with requests still in flight (storm 4, scroll 0)"). Make the per-mode
  "quit with requests in flight" assertion deterministic (e.g. quit on a signal once N requests are pending, or retry
  that mode until one run qualifies, bounded) without weakening what it proves. Show 50 consecutive passes under a
  parallel full-workspace run.
