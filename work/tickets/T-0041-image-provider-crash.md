---
id: T-0041
title: Investigate the intermittent Qt6Core access violation near the async image provider (T-0002)
type: bug
epic: E-01
status: done
priority: P0
size: S
depends_on: []
scope:
  - spikes/T-0002-cxx-qt/
owner: tw-spike
branch: t/T-0041-image-provider-crash
review_round: 0
created_by: from T-0002 (risk)
---

## Context
T-0002 saw 5 access violations in Qt6Core.dll at RVA 0xe9f73 (QObject/signal machinery) in early runs; after switching
the image-provider response to the queued-signal pattern, 0 in 13+31 runs — fix unproven, no stack captured. The
async image provider must not be reused until this is explained (ADR 0008 decision 5).

## Scope
- Build the spike with Qt debug symbols (or ASan), stress the async image provider (cancel/scroll storms, provider
  teardown at exit) and capture a stack; identify the cause; confirm the fix with a reproducer that fails before it.

## Acceptance criteria
- [ ] Root cause stated with a captured stack, or ≥ 500 stress runs with 0 crashes under a debug/ASan build plus a
      written argument why the queued pattern is lifetime-correct.
- [ ] Recommended production pattern for the image provider written into ADR 0008's notes (lead applies).

## Signals & storage
None.

## Outcome
2026-11-02: teardown crash reproduced and fixed (0/2040); 0xe9f73 not reproduced. ADR 0008 addendum; follow-up T-0044. Merge dcb1464.
