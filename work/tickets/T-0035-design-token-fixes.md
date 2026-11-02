---
id: T-0035
title: Design-system §3 token fixes from T-0011 (accentFill, onAccent, contrast, selected, alpha hex)
type: feature
epic: E-01
status: ready
priority: P1
size: S
depends_on: [T-0040]
scope:
  - docs/design/design-system.md
  - apps/tinwave-desktop/qml/Theme/Theme.qml   # the production Theme (ported from T-0011 in T-0040)
  - apps/tinwave-desktop/qml/Main.qml
  - apps/tinwave-desktop/qml/Sidebar.qml
owner:
branch:
review_round: 0
created_by: from T-0011 (design ticket per design-system §9)
---

## Context
T-0011 contrast self-test (work/reports/T-0011/spike-1.md, ADR 0007): light `textFaint` 4.28:1 on `sidebar` and
4.39:1 on `selected`; light `srcDownload` 4.30:1 on `sidebar`; `selected` hexes encode ~10 %/12 % not 12 %/18 %;
`#RRGGBBAA` values parse as `#AARRGGBB` in QML; FluentWinUI3 hard-codes the label colour on accented controls, so pale
or dark accents need an `accentFill` token; `onAccent` should be computed.

## Scope
- Fix the token table so every text pair is ≥ 4.5:1 on every surface it sits on (light and dark), with a documented
  check; add `accentFill` (accent adjusted so Fluent's fixed label colour stays ≥ 4.5:1) and computed `onAccent`;
  correct `selected` percentages; write alpha colours as `#AARRGGBB` (QML order) and say so.
- Update the prototype Theme.qml self-test to the new values (reference only; the real Theme.qml lands with the shell).

## Acceptance criteria
- [ ] The self-test passes for 15 accents × 2 schemes on every surface (≥ 4.5:1 text, ≥ 3:1 UI glyphs).
- [ ] design-system.md §3 updated; any changed look noted for the PO (token look changes are a PO escalation).

## Signals & storage
None.

## Added from P-0116 (T-0040 handoff)
Also add the tokens the shell had to work around: hairline width, default first-start window size, content-surface
corner radius (canvas rounded top-left) and the canvas sidebar row height (34) vs native ItemDelegate (40) — decide in
design-system §3/§4 and expose in Theme.qml; then replace the workarounds in apps/tinwave-desktop/qml/Main.qml
(scope extends to Main.qml/Sidebar.qml for those substitutions only).
