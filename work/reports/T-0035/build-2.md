HANDOFF T-0035 (round 2)
status: done
branch: t/T-0035-design-token-fixes   commit: 4683dd8
summary: All four review items are fixed with no token value changes. The doc and Theme comment now say `hover` is never composited over `selected`. The 15 check accents are exposed as `Theme.checkAccents`, `selfCheck()` defaults to them, and §3.1 lists the 15 hexes. The focus-ring comment is aligned with `accentText`, and the first-start window is clamped to the available screen.

fixes:
1. (blocking) Hover over selected, option (a), docs and comments only.
   - design-system §3.1 contrast rule: "`hover` is never composited over `selected`: a selected Tw row shows `selected` only, hovered or not (native `ItemDelegate` keeps its own states), so there is no hover-on-selected surface."
   - §5 states line: "A selected row shows `selected` only: `hover` is never composited over it (§3.1)."
   - Theme.qml `surfacesOf` comment states the same.
2. `Theme.checkAccents` (readonly var, the 15 T-0011 hexes) is the default for `Theme.selfCheck()`. The §3.1 "The check" bullet lists all 15 hexes and tells you to log `Theme.selfCheck().rows`. T-0045 can call it without arguments.
3. Theme.qml focus-ring comment now reads "2 px ring in `accentText` (>= 3:1 with any accent), offset 2".
4. First-start clamp (P-0134):
   - New token `windowChromeAllowance` = 48 (title bar + frame).
   - New `Theme.firstStartWidth(availableWidth)` and `Theme.firstStartHeight(availableHeight)`. Each is max(min, min(default, available [- allowance for height])), so it is never below the window minimum.
   - Main.qml uses `Theme.firstStartWidth(Screen.desktopAvailableWidth)` and `Theme.firstStartHeight(Screen.desktopAvailableHeight)`. No raw numbers outside Theme.
   - §3.3 documents it.
   - A 1366 x 768 screen at 100 % now gets about 1200 x 720.

acceptance criteria (unchanged from round 1, re-verified):
- [x] Self-check passes for 15 accents x 2 schemes on every surface — proof: scratch harness (not committed; it calls `Theme.selfCheck()` with no arguments, run with `qml -I . check.qml`) printed 30 rows and `SELFCHECK failures=0`.
- [x] design-system.md §3 updated, look changes noted — the round-1 list stands, no new look changes.

checks: fmt pass, clippy pass (-D warnings), tests 402 passed / 0 failed (6 skipped, nextest), deny pass (advisories, bans, licenses and sources ok)
files changed: docs/design/design-system.md, apps/tinwave-desktop/qml/Theme/Theme.qml, apps/tinwave-desktop/qml/Main.qml
signals added/changed: none
ui: no new screenshots needed. Light shell recaptured as a check: still 1804x1247 at 150 %, identical to round 1, so the clamp does not change the default window on this machine (2560x1440 at 150 %). Capture: scratchpad\light-r2.png. The round-1 screenshots stand.
docs updated: docs/design/design-system.md
deviations from ticket: none beyond round 1. The one new token, `windowChromeAllowance`, was added for item 4 because the title-bar room cannot be a raw number in Main.qml.

proposals: none new (the round-1 proposals stand; P-0133 canvas sync is already tracked).

questions for lead: none (the round-1 questions on ADR 0007 annotation and PO escalation stand; answered in build-1.md).

Cleanup: every tinwave-desktop process I started was killed. No Windows settings were changed. Nothing was pushed.
