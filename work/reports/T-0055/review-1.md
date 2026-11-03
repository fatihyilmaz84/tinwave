REVIEW T-0055 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass, clippy n/a, tests n/a, deny pass. `git merge-tree --write-tree main t/T-0055-canvas-prose-and-render-harness` clean (tree 10b2b43). Worktree at 373b8d7, clean.

acceptance criteria:
- [x] Main.dc.html prose matches §3.2/§3.3 and D-0006 — header "Qt 6.10+" (§2 line 47 / D-0006); Base note: active style's control font via `Theme.base` from a hidden `Label`, never `Qt.application.font` (§3.2 119-121); Layout list has sidebarRowHeight 34, hairline 1, windowMin 880 × 560, windowDefault 1200 × 800 (§3.3 138-144); danger #B42318 / #F2877D (§3.1 line 90). All visible in the reviewer's 1440×2500 Main render; no "6.8" or "10 pt" left.
- [x] Harness renders all seven artboards — `node render-canvas.js <worktree>/docs/design/canvas <scratch>` exit 0, 7 HTML in canvas.json order, no `{{` or `<sc-` left. Screenshotted Main (`--height=2500`) and Album-macOS: correct. Failure path: `{{c.hx}}` copy exits 1 ("...Main.dc.html: {{c.hx}} has no value in renderVals()"), no output. No `tinwave-canvas-*` profile left in %TEMP%, no headless msedge left.

blocking findings:
- docs/design/tools/render-canvas.js:71-80 (render) and README.md:9-11 — "no network / makes no network requests" is false for the screenshot path: docs/design/canvas/Radio-Linux.dc.html:11 has `<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Noto+Sans...">`; render() keeps it, so `--screenshot` (the documented command) makes headless Edge fetch from fonts.googleapis.com (`--disable-background-networking` doesn't block page loads); opening the HTML does the same. Breaks the "local files only" contract and makes the Radio-Linux render depend on network access. Fix in scope: render() removes (with a printed note) or rejects any `<link>`, `@import` or `url()` pointing at http(s), or also pass `--host-resolver-rules="MAP * ~NOTFOUND"`; README says remote font links are dropped. Radio-Linux.dc.html itself is out of scope (propose separately).

design check: n/a; canvas content checked matches §3.1–§3.3.

non-blocking findings:
- render-canvas.js:117-119 — success judged by `fs.existsSync(pngFile)`, `r.status` ignored: a stale `<Name>.png` from an earlier run makes a failed/timed-out browser run look successful. Delete the PNG before spawning; fail on non-zero status.
- render-canvas.js:22-25 — a missing or non-matching `renderVals()` block returns `{}` silently (low risk: `{{...}}` still fails loudly later).

proposals:
- P: Remove the Google Fonts `<link>` from Radio-Linux.dc.html | why: last remote fetch in docs/design; "nothing leaves the device" | size: S
- (builder's P-0226..P-0228 fine as lead doc fixes after merge)

notes for lead: apart from the network finding the branch is ready. Round 2: remote-resource handling + one README sentence. Scratch output deleted; nothing running.

---
Lead (2026-11-03): round 2 = drop remote http(s) resources in render() with a printed note AND pass the host-resolver
block to the browser (belt and braces), README sentence, delete-before-screenshot + non-zero status check. P-0229
(remove the Google Fonts link from Radio-Linux.dc.html) merged into round 2 — scope widened by the lead to that one file.
