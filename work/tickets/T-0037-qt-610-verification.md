---
id: T-0037
title: Re-run the T-0011 harness on a Qt 6.10 MSVC kit (the new floor)
type: chore
epic: E-01
status: blocked
priority: P2
size: S
depends_on: []
scope:
  - spikes/T-0011-native-styles/results/
owner:
branch:
review_round: 0
created_by: from T-0011 (D-0006 raised the floor to 6.10)
---

## Context
Only Qt 6.12 is installed; the floor is 6.10 (D-0006). The harness is QT_VERSION-guarded to build on older kits.

## Acceptance criteria
- [ ] Matrix + contrast self-test + screenshots on 6.10; differences from 6.12 listed.

## Notes
Blocked on the PO installing a Qt 6.10 MSVC kit (optional; can wait until before E-01 acceptance).
