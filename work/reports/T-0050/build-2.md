HANDOFF T-0050 (round 2)
status: done
branch: t/T-0050-nav-focus-ring   commit: 196123a
summary: Nav keys handled in `Keys.onPressed` with no persistent flag (fixes the stuck-`keyboardMove` blocker). Home/End/PageUp/PageDown work. capture.ps1 parks the cursor outside the window and skips the Alt tap when Tinwave is already in front. Both montages retaken (light + dark, 150 %), 7 steps each.

How the blocker is fixed (Sidebar.qml):
- `keyNavigationEnabled` false; `keyboardMove` gone. `Keys.onPressed` handles Up/Down/PageUp/PageDown/Home/End with no modifiers, `event.accepted = true`.
- `moveTo(target)` clamps the index and returns if unchanged; otherwise `positionViewAtIndex`, then `itemAtIndex(to).forceActiveFocus(Qt.TabFocusReason)`, then sets `currentIndex`.
- Single focus change: the target row is focused first with the Tab reason; when `currentIndex` changes, ListView finds the current row already focused and does nothing more. No `focus = false` dance; one focus change per key press (Narrator not run).
- A mouse click only sets `currentIndex` through `onClicked`, never through `moveTo`, so no ring.
- Page size: `max(1, floor(height / Theme.sidebarRowHeight) - 1)`; Theme token only.
- Delegate `focusPolicy` guard (`isCurrentItem || activeFocus`) stays, so an external programmatic `currentIndex` change won't bring back the "Cannot set activeFocusOnTab" warning.

acceptance criteria:
- [x] Keyboard walkthrough (light + dark, 150 %): Tab in, Down x3, Up x1, End, Home — ring on the current row in all 7 steps of both montages (Songs, Albums, Artists, Genres, Artists, Settings, Songs), checked by eye. Extra scratch captures: PageDown from Albums → Settings, PageUp → Songs, ring each time.
- [x] Mouse click shows no ring — probe (scratch capture.ps1 copy `capclick2.ps1` with keybd_event key-down/up and mouse_event): Tab x3, hold Down, Tab, release Down, click Genres. After hold/Tab/release: Albums selected, ring on the menu button. After the click: Genres selected, no ring (reviewer's probe B now clean). Round-1 click checks still hold (clicks never pass through the key handler).
- [x] Token grep clean; startup tests pass; fmt, clippy, tests, deny pass — new QML has only Theme tokens and literal 0/1 in clamps.
- [x] P-0163 foreground check kept; the only change is an early return when already foreground (forced-failure path not re-run; unchanged).
- [x] P-0181 cursor parked at the first screen corner outside the window rect before the first foreground check; no hover fill in any new panel.
- [x] P-0180 Home/End in both montages; PageDown/PageUp in scratch captures.

checks: fmt pass, clippy pass, tests 495 passed / 0 failed (6 skipped), deny pass
files changed: apps/tinwave-desktop/qml/Sidebar.qml; apps/tinwave-desktop/tools/capture.ps1; apps/tinwave-desktop/docs/screenshots/keyboard-walkthrough-windows11-fluentwinui3-{light,dark}.png
signals added/changed: none
ui: design-system §5, §7; canvas "Library · Windows" sidebar. Native ItemDelegate/ListView only; no Tw*, no new tokens, no raw values. Windows 11 26200, FluentWinUI3, 150 %, `--color-scheme` only. Fusion and non-default accents not checked. Deviations from the design: none.
capture.ps1: `Get-Foreground` returns immediately when already foreground (no Alt tap); cursor parked once before the first check; header comment updated.
deviations from ticket: the two PNGs are outside the literal scope (evidence files, as in round 1).

proposals:
- P: capture.ps1 `-Click x;y` and key-down/up steps (probes needed a scratch copy) | size: S
- P: Shared `moveTo`-style focus-follows-selection helper for screen lists (event-scoped, as P-0176) | size: S

questions for lead: none. No processes left, no settings changed, no target copies.

---
Lead (2026-11-03): P-0182 (click/key-down steps) merged into P-0175 (parked); P-0183 duplicate of P-0176 (parked, now
pointing at `moveTo`).
