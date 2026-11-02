---
id: T-0046
title: Sync the design canvas token sheets with design-system §3.1 (post-T-0035 values)
type: chore
epic: E-01
status: ready
priority: P2
size: S
depends_on: [T-0035]
scope:
  - docs/design/canvas/
owner:
branch:
review_round: 0
created_by: from P-0133
---

## Context
T-0035 changed light textFaint, srcDownload, warning, dark textFaint and the `selected` hexes, and fixed alpha order
(`#AARRGGBB` in QML). The canvas files (Main.dc.html:199-208, Components.dc.html, Library-Windows.dc.html,
Album-macOS.dc.html, others) still use #6E6E77, #E6F1F2, #2E7D46, #9A5B00, #8C8C95, #1B2B2D and CSS-order alpha.

## Scope
- Replace every token value in the canvas files with the §3.1 value; keep CSS alpha in CSS order (`#RRGGBBAA`) but
  matching the QML value. No layout changes.

## Acceptance criteria
- [ ] Grep for each old hex in docs/design/canvas/ returns nothing; a table in the handoff maps old → new per file.
- [ ] Canvas files still open and render (spot-check screenshot of Main and Library-Windows).

## Signals & storage
None.
