---
id: T-0036
title: Windows platform bridge — reduced motion, high contrast, transparency, dark menu mode, Mica (WindowEffects)
type: feature
epic: E-01
status: in_review
priority: P1
size: M
depends_on: [T-0040]
scope:
  - apps/tinwave-desktop/src/platform/      # windows.rs behind the WindowEffects / platform traits (the only unsafe)
  - apps/tinwave-desktop/src/app/           # the QML bridge file (bridge files must share src/app/, ADR 0008) + wiring
  - apps/tinwave-desktop/qml/Theme/Theme.qml  # inputs: reduced motion, high contrast, transparency, accent
  - apps/tinwave-desktop/qml/Main.qml       # Mica / solid fallback, --accent dev switch (P-0132)
  - apps/tinwave-desktop/qml/Sidebar.qml    # the app Menu popupType only (P-0121)
  - apps/tinwave-desktop/Cargo.toml         # `windows` crate features only
  - apps/tinwave-desktop/tests/
  - apps/tinwave-desktop/README.md          # dev switches
owner: tw-builder
branch: t/T-0036-windows-platform-bridge
review_round: 1
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

## Added from P-0132 (T-0035 build-1) and lead grooming (2026-11-03)
- Dev switch `--accent=#rrggbb` beside `--color-scheme` (debug and release, like `--color-scheme`; never touches the OS),
  so UI tickets can screenshot pale/dark accents for the Definition of Done.
- Scope widened at grooming: the bridge needs a file in src/app/ (ADR 0008: one directory per QML module), wiring, the
  Theme inputs, Main.qml for Mica, Sidebar.qml for the menu popupType.
- Acceptance additions: screenshots light + dark at 150 % with Mica on and with transparency off (solid fallback);
  a non-default accent via `--accent`; the dark native menu (or the Popup.Window fallback) open in dark mode.
- Keep it M: if it grows past that, stop and propose a split (e.g. Mica/menus vs settings providers).
