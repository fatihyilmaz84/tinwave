REVIEW T-0050 (round 2)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass, clippy pass (-D warnings), tests pass (nextest: 495 run, 495 passed, 6 skipped; cancel_storm and both startup tests passed), deny pass
merge: `git merge-tree --write-tree main t/T-0050-nav-focus-ring` clean (tree 80c8476). Worktree clean at 196123a. Round-2 diff: Sidebar.qml, capture.ps1, the two PNGs. Nothing under work/.

acceptance criteria:
- [x] Keyboard walkthrough — both 7-step montages show the ring on the current row at every step (Songs, Albums, Artists, Genres, Artists, Settings, Songs), no hover fill. Reviewer's own `capture.ps1 -Scheme light -Keys "{TAB}x3,{DOWN}x3,{UP},{END},{HOME}"` ends on Songs with the ring, cursor parked.
- [x] Mouse click shows no ring — probe B (Tab x3, Down key-down only, Tab, Down released): Albums selected, ring on the menu button; click Genres → selected, no ring (rev50r2\B2-click.png). Probe A also clean. Nothing persists between key events.
- [x] Token grep clean (only Main.qml:42 `spacing: 0`, Sidebar.qml:70 `currentIndex: 0`); page formula uses `Theme.sidebarRowHeight` plus literal 0 and 1.
- [x] P-0163/P-0181 capture.ps1 — forced-failure scratch copy: 5 retry warnings, "capture aborted … Nothing was saved", exit 1, no PNG, no process. Cursor parked at (2559,0). Early return skips the Alt tap when already in front.

Also verified (QT_FORCE_STDERR_LOGGING=1, every .err empty — no "Cannot set activeFocusOnTab"): End → Settings; End then Home → Songs; PageDown → Settings; PageDown then PageUp → Songs; Up at the top stays on Songs; ring each time. Shift+Down and Ctrl+Down passed on (selection stays, ring kept). Shift+Tab then Tab back to the current row with the ring; Tab from a row goes to the menu button (one tab stop). At minimum window height (~563 logical) PageDown, End, End+PageUp, Down+PageDown+Up all scroll the target into view with the ring.

blocking findings:
- apps/tinwave-desktop/qml/Sidebar.qml:93 — `if (event.modifiers !== Qt.NoModifier) return` also drops arrow keys carrying `Qt.KeypadModifier`; with `keyNavigationEnabled: false` (line 78) nothing else handles them. Windows (reproduced): numpad arrows (non-extended VK_DOWN, scancode 0x50, NumLock off) twice after Tab x3 — main moves to Artists, branch stays on Songs (rev50r2\montage-M9.png). macOS (Qt 6.12 docs, not run): KeypadModifier is set on every arrow key, so Up/Down would do nothing. Regression vs main. Fix: ignore only real modifiers (`if (event.modifiers & ~Qt.KeypadModifier) return`, or test Shift|Control|Alt|Meta); re-check numpad Down/Up and that Shift/Ctrl+Down are still not handled.

design check: passes apart from the finding. Native ItemDelegate/ListView only; no new tokens or Tw*; accessible names kept; native ring light (black) and dark (white); montages match.

non-blocking findings:
- Sidebar.qml:96 — page size counts only rows, not section headers, and keeps one row less of context; at default and minimum height that is ≥ 12 rows against 12 items, so PageDown/PageUp act like End/Home (clamped correctly). Optional: target from what is visible (`indexAt(…, contentY + height - 1)`).
- capture.ps1:94-99 — the cursor is never put back; if the window covers all four corners (maximized), no corner qualifies and the cursor is silently not parked.

proposals:
- P: Sidebar PageUp/PageDown by the rows actually visible (section headers included) | size: S
- P: capture.ps1 restores the cursor and warns when it can't park it outside the window | size: S

notes for lead: merge clean; blocker is a one-line fix; montages need no retake (dedicated arrows already work). P-0176 should carry the same KeypadModifier rule. All processes killed, no settings changed, cursor restored. Evidence: scratchpad\rev50r2\ (B1/B2/A pngs, montage-P/M9/U, cap-light-walk.png, ff\capture-ff.ps1); probe: scratchpad\probe2.ps1. montage-T.png and S*.png are invalid runs.

---
Lead (2026-11-03): round 2 failed on a one-line fix → lead takeover: Sidebar.qml now ignores only real modifiers
(`event.modifiers & ~Qt.KeypadModifier`), with a comment. Verification below. P-0185 (visible-rows paging) and P-0186
(capture cursor restore/warn) parked → next sidebar / capture touch. P-0176 updated: carry the KeypadModifier rule.

Lead takeover result (2026-11-03): 3b60319. Verified with the reviewer's probe (scratchpad\probe2.ps1) on the branch
debug build: Tab x3 + non-extended numpad Down (VK 0x28, scan 0x50) x2 → Artists with the ring (lead-numpad.png);
Tab x3, Down, Shift+Down, Ctrl+Down → stays on Albums with the ring (lead-mods.png); stderr empty for both; no process
left. Merged fc1bfc8.
