---
id: T-0036
title: Windows platform bridge — reduced motion, high contrast, transparency, dark menu mode, Mica (WindowEffects)
type: feature
epic: E-01
status: ready
priority: P1
size: M
depends_on: [T-0040]
scope:
  - apps/tinwave-desktop/src/platform/      # windows.rs behind the WindowEffects / platform traits
owner:
branch:
review_round: 0
created_by: from T-0011
---

## Context
ADR 0007 / D-0006: SetPreferredAppMode (dark native menus, fallback Popup.Window), Mica with
QT_QPA_DISABLE_REDIRECTION_SURFACE=1 and solid fallback, SPI_GETCLIENTAREAANIMATION / SPI_GETHIGHCONTRAST with
WM_SETTINGCHANGE updates (SendMessage reached the filter; PostMessage did not), Theme bridge inputs. Prototype:
spikes/T-0011-native-styles/host/.

## Acceptance criteria
- [ ] Each provider unit-tested; live OS toggles covered by the PO runbook (spikes/T-0011-native-styles/PO-RUNBOOK.md).
- [ ] unsafe only in one documented module with per-block SAFETY comments; non-Windows returns Unsupported.

## Signals & storage
None (theme/accent changes feed no signal without a catalog entry).

## Added from P-0121 (T-0040 review)
Set the app Menu (apps/tinwave-desktop/qml/Sidebar.qml) to `Popup.Native` with the SetPreferredAppMode dark opt-in, falling back to `Popup.Window` (ADR 0007 decision 1); scope extends to that one property.
