---
id: T-0055
title: Canvas — refresh stale prose in Main.dc.html and add a local render harness
type: chore
epic: E-01
status: in_progress
priority: P2
size: S
depends_on: [T-0046]
scope:
  - docs/design/canvas/Main.dc.html
  - docs/design/canvas/Radio-Linux.dc.html   # round 2: drop the remote font link only (P-0229)
  - docs/design/tools/            # new: the render harness and a short README
owner: tw-builder
branch: t/T-0055-canvas-prose-and-render-harness
review_round: 2
created_by: from P-0213, P-0214 (T-0046 build-1)
---

## Context
Main.dc.html still says "Base = Qt.application.font" (design-system §3.2: `Theme.base` reads the style's control
font), "Qt 6.8 LTS" (minimum is 6.10, D-0006), and its Layout card lacks `sidebarRowHeight` 34, `hairline` and the window
default size. The .dc.html files need the claude.ai canvas runtime (`./support.js`, not in the repo), so agents can't see
them rendered; T-0046's builder wrote a ~25-line Node harness that expands `<sc-for>` and `{{path}}`.

## Scope
- Fix the three prose items in Main.dc.html to match design-system.md.
- Add the missing `danger` swatch (#B42318 / #F2877D, §3.1) to the Main.dc.html swatch sheet (T-0046 review-1).
- docs/design/tools/render-canvas.js (+ README): renders a .dc.html to plain HTML from local files only; document how to
  screenshot it with a local headless browser. Node is a dev-only prerequisite; say so.

## Acceptance criteria
- [ ] Main.dc.html prose matches design-system.md §3.2/§3.3 and D-0006.
- [ ] The harness renders all seven artboards; one screenshot each listed in the handoff.

## Signals & storage
None.
