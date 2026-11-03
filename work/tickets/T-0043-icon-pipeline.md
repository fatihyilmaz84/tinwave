---
id: T-0043
title: Icon pipeline — Lucide SVGs in the Qt resource file, TwIcon component, sidebar and menu glyphs
type: feature
epic: E-01
status: in_review
priority: P1
size: M
depends_on: [T-0040]
scope:
  - apps/tinwave-desktop/qml/Tw/TwIcon.qml
  - apps/tinwave-desktop/icons/
  - apps/tinwave-desktop/tools/
  - apps/tinwave-desktop/build.rs        # add the icon resources only
  - apps/tinwave-desktop/qml/Sidebar.qml # use icons on the menu button and nav items
  - docs/design/design-system.md        # §3.4 / §5 if the component contract needs wording
  - THIRD-PARTY-NOTICES or the app's notices source (Lucide ISC text)
owner: tw-builder
branch: t/T-0043-icon-pipeline
review_round: 1
created_by: from T-0040 (P-0115)
---

## Context
design-system §3.4: one stroke set, Lucide (ISC), SVG in the Qt resource file, 20 px at 1.75 stroke (16 px dense),
tinted with `text`/`textMuted`; filled glyphs only for "on" states; no emoji or icon fonts. T-0040 shell shows the word
"Menu" instead of the hamburger and no nav icons.

## Scope
- Vendor only the Lucide SVGs used (pinned version, ISC licence text kept), compiled into the QML module resources.
- `TwIcon` (name, size token, colour token; tints via the Qt Quick `IconImage`/`icon` mechanism so native controls keep
  their own icon handling; crisp at 100/150/200 % scaling).
- Use it for the app-menu button (with accessible name kept) and the sidebar nav items.

## UI & design
- Design reference: design-system §3.4, §4; canvas "Library · Windows" sidebar.
- Native controls keep `icon.source`/`icon.color`; TwIcon only where a native control has no icon slot.

## Acceptance criteria
- [ ] Screenshots light + dark at 150 %, icons tinted per token, no blur.
- [ ] Token grep clean; no raw sizes/colours.
- [ ] Unused icons are not shipped (test or build check lists the vendored set).
- [ ] Lucide licence text included in the notices source.
- [ ] fmt, clippy, tests, deny pass.

## Signals & storage
None.

## Added from P-0120 (T-0040 review)
Add a small screenshot capture helper (apps/tinwave-desktop/tools/) that crops to DWMWA_EXTENDED_FRAME_BOUNDS so captures have no black resize-border strips; use it for this ticket's screenshots.

## Added from P-0131 (T-0035 build-1)
Drive keyboard steps with the window active (the focus ring is not drawn in an inactive window under PrintWindow).
Starting point: the T-0035 builder's scratch scripts cap.ps1 / cap2.ps1 in the lead session scratchpad
(C:\Users\FSYY\AppData\Local\Temp\claude\D--dev-Tinwave\53bf9027-c019-4dd4-82f1-5a72c6705b0c\scratchpad\).
