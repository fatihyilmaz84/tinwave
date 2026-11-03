HANDOFF T-0055 (round 1)
status: done
branch: t/T-0055-canvas-prose-and-render-harness   commit: 373b8d7
summary: Main.dc.html prose matches design-system §3.2/§3.3 and D-0006, and the swatch sheet has the `danger` swatch. Added docs/design/tools/render-canvas.js (+ README.md): renders a .dc.html or the whole canvas folder to plain HTML from local files only, and can screenshot with a local headless Edge/Chrome. All seven artboards render and were screenshotted. Docs and tooling only.

acceptance criteria:
- [x] Main.dc.html prose matches §3.2/§3.3 and D-0006 — header "Qt 6.8 LTS" → "Qt 6.10+"; type note: Base is the active style's control font, read by `Theme.base` from a hidden `Label`, never `Qt.application.font` (Segoe UI Variable 14 px, SF Pro 13, Linux desktop setting); Layout card gained `sidebarRowHeight 34`, `hairline 1`, `windowDefault 1200 × 800` (windowMin kept); danger swatches #B42318 (light), #F2877D (dark). Main-tall.png shows 11 layout rows and `danger` in both sheets.
- [x] The harness renders all seven artboards — `node docs/design/tools/render-canvas.js docs/design/canvas <out> --screenshot` exit 0, 7 html + 7 png in scratchpad\t55\ (Main, Components, Platforms, Library-Windows, Album-macOS, Radio-Linux, SignalCenter-Windows); extra scratchpad\t55tall\Main.png (1440x2500). Failure path: `{{c.hex}}` → `{{c.hx}}` in a copy of Main.dc.html exits 1 with "{{c.hx}} has no value in renderVals()".

checks: fmt pass, clippy n/a, tests n/a, deny pass
files changed: docs/design/canvas/Main.dc.html; docs/design/tools/render-canvas.js (new); docs/design/tools/README.md (new)
signals added/changed: none
ui: none. Node v20.19.6; installed Microsoft Edge, headless. Nothing installed.
deviations from ticket: none. Extras: the harness errors loudly on unresolved `{{path}}`, non-array lists or unsupported `<sc-*>` tags; screenshots use a throw-away profile with `--disable-sync --disable-background-networking --disable-component-update` (Edge otherwise tried a sync call); `--height=N` override (the only way to see all of Main).

proposals:
- P: Main board shorter than its content (canvas.json h=1900; content ends ~y=2300) → canvas.json h and Main's `$preview` height ~2400 | size: S
- P: Platforms.dc.html:102 still says "FluentWinUI3 (Qt 6.8+)" | size: S
- P: Radius table in Main.dc.html lacks the `content` row (`radiusContent` = `surface`, §3.3) | size: S

questions for lead: none

---
Lead (2026-11-03): the three proposals (P-0226..P-0228) are one-line doc fixes → lead applies them on main after the
merge.
