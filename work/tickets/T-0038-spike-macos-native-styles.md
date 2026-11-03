---
id: T-0038
title: "Spike: macOS native styles on real hardware (macOS style, vibrancy, global menu bar, native dialogs and alerts, ExpandedClientAreaHint)"
type: spike
epic: E-01
status: blocked
priority: P2
size: M
depends_on: []
scope:
  - spikes/T-0038-macos-native-styles/
owner:
branch:
review_round: 0
created_by: from T-0011 (D-0001 Windows first)
---

## Question
Do the design-system §2 macOS rows hold on a real macOS machine with Qt ≥ 6.10?

## Notes
Blocked: needs a macOS machine or CI runner (D-0001). Must run before E-01 acceptance.

## Note from T-0036 (2026-11-03)
- The platform bridge maps non-Windows to `Popup.Window` for the app menu until this spike verifies `Popup.Native`
  here; flip `PlatformSettings.nativeMenus` in the platform impl when it does (P-0188: add QML probe coverage).
- (P-0216, T-0053) `platform::has_native_window_handles(qpa)` returns true on non-Windows today; narrow it to this
  OS's native QPA plugin names when its platform impl lands.
