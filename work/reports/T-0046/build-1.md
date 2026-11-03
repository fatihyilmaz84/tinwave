HANDOFF T-0046 (round 1)
status: done
branch: t/T-0046-canvas-token-sync   commit: 766d958
summary: Replaced every stale post-T-0035 token hex in docs/design/canvas/ (six old values, 34 line edits in 6 files) with the design-system §3.1 / Theme.qml values, and set the Windows canvases' sidebar nav icons and app-menu icon from 18 to 20 px (P-0158). CSS alpha values were already numerically equal to the QML ones, so they stayed in CSS order. (The worktree didn't exist when the builder started; it was created on the existing branch at 352600e.)

acceptance criteria:
- [x] Grep for each old hex returns nothing; old → new table per file — `grep -rniE "6E6E77|E6F1F2|2E7D46|9A5B00|8C8C95|1B2B2D" docs/design/canvas/` → no output, exit 1.
- [x] Canvas files still render (Main, Library-Windows) — the .dc.html files load `./support.js` (claude.ai canvas runtime, not in the repo), so a plain browser shows the raw template; a throwaway Node harness (scratchpad, local files only) evaluates `renderVals()` and expands `<sc-for>` and `{{path}}`; screenshotted with local headless Edge. Main swatches show #E7F1F2, #64646D, #2A7240, #925700 (light), #1E2B2E, #94949C (dark); Library-Windows nav icons visibly 20 px.
- [x] P-0158: Library-Windows.dc.html lines 27 (app menu) and 36 (nav item template), SignalCenter-Windows.dc.html lines 27 and 36 now `<svg width="20" height="20"`.

Token mapping: #6E6E77 → #64646D (light textFaint); #8C8C95 → #94949C (dark textFaint); #E6F1F2 → #E7F1F2 (light selected); #1B2B2D → #1E2B2E (dark selected); #2E7D46 → #2A7240 (light srcDownload); #9A5B00 → #925700 (light warning).
- Main.dc.html: #6E6E77 x3 (39, 50, 201); #8C8C95 x1 (208); #E6F1F2 x1 (199); #1B2B2D x1 (206); #2E7D46 x1 (202); #9A5B00 x1 (202)
- Components.dc.html: #6E6E77 x7 (26, 101, 134, 139, 223, 230 x2); #E6F1F2 x3 (70, 228, 234); #2E7D46 x2 (115, 228); #9A5B00 x1 (235)
- Library-Windows.dc.html: #2E7D46 x1 (139); icons 18 → 20 (27, 36)
- Album-macOS.dc.html: #8C8C95 x8 (25, 46, 58, 81, 86, 91, 94, 157)
- Radio-Linux.dc.html: #E6F1F2 x2 (164)
- SignalCenter-Windows.dc.html: #8C8C95 x1 (103); #1B2B2D x1 (170); icons 18 → 20 (27, 36)
- Platforms.dc.html, canvas.json: no change.
Alpha tokens: Main swatches `hover` #0000000A / #FFFFFF0F and `separator` #0000001A / #FFFFFF14 equal the QML values in CSS order; other 8-digit alphas are native-control or accent illustrations, not tokens.

Screenshots (headless Edge on the harness render): scratchpad\Main-after.png, scratchpad\Library-Windows-after.png; harness scratchpad\render.js.

checks: fmt pass, clippy pass, tests 585 passed / 0 failed (6 skipped), deny pass
files changed: docs/design/canvas/{Main,Components,Library-Windows,Album-macOS,Radio-Linux,SignalCenter-Windows}.dc.html
signals added/changed: none
ui: design-reference sync only; no QML touched
deviations from ticket: none. Judgement calls: 18 → 20 only on the Windows sidebars (nav items, app-menu button); transport icons left (18; play button 20); Album-macOS and Radio-Linux 16 px nav icons (compact 28 px rows) left; only the six stale hexes changed.

proposals:
- P: Align the remaining near-miss canvas surface colours with §3.1 (Library-Windows #EDEFF3 title bar/sidebar vs #ECECEF, #F7F8FA scan card, #E1E4EA selected row, #E5E5E8/#E4E4E8 hairlines; SignalCenter-Windows #1C1C1F root/transport vs #161618, #202024 title bar/sidebar vs #1D1D20; Album-macOS #232326 sidebar, #1D1D20 transport) | size: S
- P: Decide the macOS and Linux sidebar nav icon size (16 px in 28 px rows vs `iconSize` 20) | size: S
- P: Refresh stale prose in Main.dc.html ("Base = Qt.application.font" vs §3.2; "Qt 6.8 LTS" vs 6.10 minimum; Layout card lacks `sidebarRowHeight` 34, `hairline`, window default size) | size: S
- P: Ship a tiny local render harness for the canvas (~25 lines; .dc.html needs the claude.ai runtime) | size: S

questions for lead: none

---
Lead (2026-11-03): P-0211 (near-miss surfaces) and P-0212 (macOS/Linux nav icon size) → D-0008 question for the PO
(the canvas is the PO's artboard; tokens win in code, but whether the artboards change is a design call). P-0213 (stale
prose) accepted → T-0055 together with P-0214 (local render harness, scope docs/design/tools/).
