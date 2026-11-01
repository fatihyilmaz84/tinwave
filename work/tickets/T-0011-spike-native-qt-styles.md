---
id: T-0011
title: "Spike: native Qt Quick styles and OS integration for the Quiet Native design system"
type: spike
epic: E-00
status: ready
priority: P1
size: M
depends_on: []
scope:
  - spikes/T-0011-native-styles/
owner:
branch:
review_round: 0
created_by: lead
---

## Context
ADR 0003 and docs/design/design-system.md commit to native Qt Quick Controls styles per OS, system font/accent/
color scheme, native menus and dialogs, and a token-driven `Theme.qml`, with Qt 6.8 LTS as the floor.
Spec §2, §12 (memory), §14.1. Rules: docs/rules/qml-ui.md. Visual reference: the PO's design canvas
(Foundations, Components, Native per platform).

## Question
Does Qt 6.8 give us the native look and OS integration the design system assumes, at acceptable memory cost,
and what is the minimal correct `Theme.qml` shape?

## Scope
- Standalone QML prototype (may reuse T-0002's bridge setup if available, but must not depend on it).
- Style selection at startup: FluentWinUI3 on Windows; record what `macOS` and `Fusion` give via
  `QT_QUICK_CONTROLS_STYLE` where runnable, and note what must be verified on real macOS/Linux machines (D-0001).
- Read `palette.accent`, `Qt.styleHints.colorScheme`, `Qt.application.font`; react live to OS theme/accent changes;
  apply a user override with `setColorScheme`.
- `Theme.qml` singleton prototype with the tokens from design-system.md §3, including `accentText` contrast
  adjustment (≥ 4.5:1) for arbitrary OS accents.
- Native popups: `Menu` with `popupType: Popup.Native`, `FileDialog`/`FolderDialog`, `MessageDialog`.
- Windows: Mica behind the sidebar via `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)` with a solid fallback;
  measure the cost.
- Windows reduced-motion and high-contrast detection.
- Measure RSS and startup with each style vs plain `Basic` style.

## Out of scope
- Media-session integration (SMTC/MPRIS/Now Playing) — separate ticket after T-0003.
- Real `Tw*` components.

## Time box
1 working session.

## Success criteria
- Recommendation: confirm or amend ADR 0003 (Qt floor, style per OS, Mica), with measurements.

## Acceptance criteria
- [ ] SPIKE RESULT with RSS/startup per style on the dev machine, and a table of which design-system.md §2 rows
      are confirmed, need a workaround, or are unsupported.
- [ ] Prototype `Theme.qml` in `spikes/T-0011-native-styles/` covering every token in design-system.md §3.
- [ ] Screenshots of one native-controls page in light + dark with two different OS accents.
- [ ] Draft ADR (amendment or confirmation of 0003) at `spikes/T-0011-native-styles/ADR-draft.md`.
- [ ] Qt module licenses for any extra module used (Dialogs, etc.) recorded.

## Signals & storage
None. (Note whether theme/accent changes should feed context — they should not without a catalog entry.)

## UI & design
- Design reference: docs/design/design-system.md §2–§3; canvas "Foundations" and "Native per platform".
- Native controls: Button, TextField, ComboBox, Switch, CheckBox, Slider, Menu, FileDialog, MessageDialog.
- Tw components: none.
- Platform notes: Windows measured; macOS/Linux reported as "unverified" where not runnable.
