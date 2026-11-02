---
id: T-0042
title: Windows packaging — windeployqt recipe, THIRD-PARTY-NOTICES (Qt LGPL-3), installer check
type: chore
epic: E-01
status: ready
priority: P1
size: M
depends_on: [T-0040]
scope:
  - apps/tinwave-desktop/packaging/
owner:
branch:
review_round: 0
created_by: from T-0002
---

## Context
ADR 0008: dynamic Qt only; trimmed windeployqt bundle 55.4 MiB (FluentWinUI3 needs Fusion deployed); LGPL-3 texts,
notices for third-party code inside Qt (FreeType, PSL data, …), Qt source offer; never ship GPL-only test-utils.

## Acceptance criteria
- [ ] Scripted deploy produces a self-contained folder that runs with Qt off PATH.
- [ ] THIRD-PARTY-NOTICES generated (qtattributionsscanner + cargo-deny/about for crates) and included.
- [ ] A check fails the build if a GPL-only Qt module or a static Qt is present.

## Signals & storage
None.
