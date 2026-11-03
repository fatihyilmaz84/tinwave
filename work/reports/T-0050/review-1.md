REVIEW T-0050 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass (exit 0), clippy pass (-D warnings, exit 0), tests pass (nextest: 495 run, 495 passed, 6 skipped; cancel_storm passed), deny pass
merge: `git merge-tree --write-tree main t/T-0050-nav-focus-ring` clean (tree 8535b78). Worktree clean at 2064da8. The branch changes 4 files: Sidebar.qml, capture.ps1, the two walkthrough PNGs. Nothing under work/.

acceptance criteria:
- [x] Keyboard walkthrough (light + dark, 150 %, capture.ps1): committed montages show the ring on Songs, Albums, Artists, Genres, Artists. My `capture.ps1 -Scheme light -Keys "{TAB},{TAB},{TAB},{DOWN},{DOWN},{DOWN},{UP}"` ends with the ring on Artists; dark runs with Tab and Down also put the ring on the current row.
- [ ] Mouse click does not show the ring — a click can show the keyboard ring. Normal use works: after Tab x3, Down, Down, Up, clicking Genres selects it with no ring (probe A). But "hold Down, press Tab, release Down, then click a row" leaves the ring on the clicked row (probe B).
- [x] Token grep clean; startup tests pass; fmt, clippy, tests, deny pass — grep finds only Main.qml:42 `spacing: 0` (untouched) and Sidebar.qml:70 `currentIndex: 0` (not a size); `tinwave-desktop::startup` (2 tests) passed.
- [x] P-0163: capture.ps1 foreground check — scratch copy with the comparison forced to fail: 5 retry warnings, "capture.ps1: capture aborted: … Nothing was saved …", exit 1, no PNG, no process left. No SystemParametersInfo or other settings API; only keybd_event Alt, AttachThreadInput, SetForegroundWindow.

Other checks, verified in the running app: still one tab stop (from the current row Tab wraps to the menu button); Shift+Tab out and Tab back lands on the current row with the ring; the "Cannot set activeFocusOnTab to false…" warning is gone (QT_FORCE_STDERR_LOGGING=1: branch exe logs nothing for Tab x3 then Down/Up; main exe logs it twice for Tab x3, Down x2).

blocking findings:
- apps/tinwave-desktop/qml/Sidebar.qml:79-93 and :149 — `keyboardMove` can stay true after the key event: it is cleared only by a key release that reaches the ListView. If focus leaves the list while a navigation key is held (Tab/Shift+Tab, Alt+Tab or any deactivation, a popup), the release goes elsewhere and the flag stays true. The next `currentIndex` change, including a mouse click on a row, goes through the Tab-reason refocus, so the click shows the keyboard ring — breaks AC 2 and design-system §7. Reproduced with real input (keybd_event, mouse_event): Tab x3, Down held (keydown only), Tab, Down released, click Genres → selected with the white keyboard ring (probe B). Latent risk: once screens change `currentIndex` from outside, a stuck flag would make `forceActiveFocus` steal focus (e.g. from Search). Fix: (a) handle Up/Down in `Keys.onPressed` (`event.accepted = true`, `decrementCurrentIndex()`/`incrementCurrentIndex()`, refocus there if the index changed; no flag), or (b) keep the flag but clear it with `Qt.callLater(() => keyboardMove = false)` in the press handler and in `onActiveFocusChanged` when `!activeFocus`. Retake the walkthrough; test hold-Down-then-Tab-then-click.

design check: findings above. Otherwise passes: native ItemDelegate/ListView only, no new tokens or Tw*, accessible names kept; native FluentWinUI3 ring in light (black) and dark (white); screenshots match my captures.

non-blocking findings:
- Sidebar.qml:85-88 — PageUp/PageDown/Home/End cases do nothing (ListView handles arrows only): `{TAB}x3,{END}` stays on Songs; `{TAB}x3,{DOWN},{END}` stays on Albums; `…{END},{HOME},{PGDN}` stays on Songs. The handoff's claim is wrong for 4 of 6 keys (the AC needs only Up/Down). Remove the dead cases or make the keys work.
- keyboard-walkthrough-…-light.png — the mouse pointer was over Artists during capture; steps 1, 2, 4 show a hover fill that looks like a second selection. Park the cursor outside the window before capturing.
- capture.ps1:67-71 — `Get-Foreground` taps Alt before every key even when Tinwave is already in front (extra Alt events in keyboard evidence). Skip activation when `GetForegroundWindow() -eq $handle`.
- Sidebar.qml:150-151 — `focus = false` then `forceActiveFocus` briefly makes the ListView the active focus item; no visible flicker, but a screen reader may announce twice (not checked with Narrator).

proposals:
- P: Home/End (and PageUp/PageDown) in the sidebar nav list | why: Windows list convention | size: S
- P: capture.ps1 parks the cursor outside the window before capturing | size: S

notes for lead: merge clean. Blocking fix is small, in Sidebar.qml; retake the PNGs (cursor parked). P-0176 should take the event-scoped approach. No processes left; no settings changed. Probes in scratchpad (probe.ps1, A-click-after-keys.png, B-stuck.png).

---
Lead (2026-11-03): round 2 = option (a) — handle the navigation keys in `Keys.onPressed`, no persistent flag — and make
Home/End/PageUp/PageDown work (P-0180 merged here; Windows list convention), avoiding the double focus change if
possible; capture.ps1 parks the cursor and skips the Alt tap when already foreground (P-0181 merged here); retake both
montages. P-0176 outcome updated: use the event-scoped approach.
