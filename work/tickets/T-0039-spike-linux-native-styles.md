---
id: T-0039
title: "Spike: Linux native styles on real hardware (Fusion + platform palette on KDE and GNOME, XDG portal dialogs, accent/scheme, Popup.Native fallback, Wayland decorations)"
type: spike
epic: E-01
status: blocked
priority: P2
size: M
depends_on: []
scope:
  - spikes/T-0039-linux-native-styles/
owner:
branch:
review_round: 0
created_by: from T-0011 (D-0001 Windows first)
---

## Question
Do the design-system §2 Linux rows hold on a real Linux machine with Qt ≥ 6.10?

## Notes
Blocked: needs a Linux machine or CI runner (D-0001). Must run before E-01 acceptance.

## Note from T-0036 (2026-11-03)
- The platform bridge maps non-Windows to `Popup.Window` for the app menu until this spike verifies `Popup.Native`
  here; flip `PlatformSettings.nativeMenus` in the platform impl when it does (P-0188: add QML probe coverage).
