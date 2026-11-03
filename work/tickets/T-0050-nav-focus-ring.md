---
id: T-0050
title: Sidebar nav list — the keyboard focus ring follows the current item after Up/Down
type: bug
epic: E-01
status: in_review
priority: P1
size: S
depends_on: [T-0043]
scope:
  - apps/tinwave-desktop/qml/Sidebar.qml
  - apps/tinwave-desktop/tools/capture.ps1   # P-0163: verify foreground before capturing
owner: tw-builder
branch: t/T-0050-nav-focus-ring
review_round: 2
created_by: from P-0154, P-0155 (T-0043 build-1), P-0163 (T-0043 build-2)
---

## Context
Since T-0040 the nav list is one tab stop, but after Up/Down the native focus ring stays on the first delegate while the
selection moves, so a keyboard user loses the ring (design-system §7). Visible in
apps/tinwave-desktop/docs/screenshots/keyboard-walkthrough-*.png.

## Scope
- Move active focus with the current item (or draw the native focus visual from `ListView.isCurrentItem` and the list's
  `activeFocus`), keeping one tab stop, native ItemDelegate visuals and the accessible names.

## UI & design
- design-system §5 (states), §7 (focus ring always visible for keyboard focus); canvas "Library · Windows" sidebar.

## Acceptance criteria
- [ ] Keyboard walkthrough screenshots (light + dark, 150 %, tools/capture.ps1): Tab into the list, Down ×3, Up ×1 — the
      ring is on the current row in every step.
- [ ] Mouse click does not show the ring (native behaviour kept).
- [ ] Token grep clean; startup tests pass; fmt, clippy, tests, deny pass.

## Signals & storage
None (navigation between sections is not a preference signal yet; browse signals arrive with the screen tickets).

## Added from P-0163 (T-0043 build-2)
- `tools/capture.ps1` checks the window is the foreground window before each keyboard capture; retry a few times
  with a short pause, then fail loudly (non-zero exit) instead of saving a capture of an inactive window.
