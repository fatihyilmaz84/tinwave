HANDOFF T-0043 (round 2)
status: done
branch: t/T-0043-icon-pipeline   commit: bd4f333
summary: All blocking and requested items fixed. Nav glyphs follow the canvas. build.rs uses `CxxQtBuilder::qrc_resources` (no generated qrc), so the crate is fresh again. Nav icons use `Theme.text` with a `Theme.m` gap. The usage scan is whitespace-tolerant and skips comments; the two stale comments are fixed.

acceptance criteria / review items:
- [x] Blocking 1, glyphs match the canvas: Songs eighth-notes; Albums disc; Artists user-round; Genres layout-grid; Smart Radio radio; Insights chart-column (unchanged); This computer folder; Downloads arrow-down-to-line; Import & organize upload; CD ripping disc-3 (distinct from Albums); Signal Center activity; Settings sliders-horizontal; menu button menu. Re-vendored with `tools/vendor-icons.ps1 -Add ...` (13 icons, all 200 at tag 1.67.0); 8 unused SVGs deleted. `icons/` = the 13 used + VERSION, LICENSE-lucide.txt, README.md.
- [x] Blocking 2, freshness: generated `$OUT_DIR/icons.qrc` and `.qrc()` gone. `build.rs` passes `QResources::new().resource(QResource::new().files(["icons/<file>", ...]))` to `qrc_resources`, plus `rerun-if-changed=icons` for added/removed files. Proof: second unchanged `cargo build -p tinwave-desktop` printed only `Finished ... in 0.17s`; after `touch icons/menu.svg` it recompiled (9.6 s), next build fresh (0.12 s). The generated qrc has prefix `/qt/qml/Tinwave/Desktop` with `alias="icons/LICENSE-lucide.txt"` and `alias="icons/menu.svg"` — the licence is still in the resources. Unescaped-path issue gone.
- [x] Item 3, tint and gap: `icon.color: Theme.text` for every nav glyph, `spacing: Theme.m`; size `Theme.iconSize`. §3.4 wording fixed (old `textMuted` for resting rows removed).
- [x] Item 4, hardened scan: a comment stripper removes `//` and `/* */` but leaves string literals; matching is `key <ws> sep <ws> "literal"` for `iconName:`, `TwIcons.source(`, and `name:` inside `TwIcon { }` blocks (brace nesting handled; `TwIcons` and `Accessible.name` excluded). Literal-only rule still documented. Checked by hand and reverted: commented `// iconName: "ghost1"` and `/* TwIcons.source("ghost2") */` caused no failure; `iconName :   "disc-3x"` caught, with unused `disc-3` and missing `disc-3x` reported.
- [x] Item 5, comments: `TwIcons.qml` and `tools/vendor-icons.ps1` point at `check_icon_usage` in build.rs.
- [x] Screenshots recaptured with `tools/capture.ps1`.

checks: fmt pass, clippy pass (-D warnings), tests 495 passed / 0 failed (6 skipped; startup, `window_shell_loads_without_qml_errors`, image_provider included), deny pass

files changed (round 2; apps/tinwave-desktop/ unless noted): build.rs; qml/Sidebar.qml; qml/Tw/TwIcons.qml; tools/vendor-icons.ps1; icons/ (8 SVGs deleted, 8 added); docs/design/design-system.md (repo root)

signals added/changed: none

ui:
- Design reference: canvas Library-Windows (nav glyphs, `Theme.text` tint, 12 px gap). Windows 11, FluentWinUI3, 150 %, light and dark.
- Screenshots in scratchpad\t0043\evidence-r2\: shell-windows11-fluentwinui3-light-150.png, shell-windows11-fluentwinui3-dark-150.png, kb-1-menu-button-focus-windows11-fluentwinui3-light-150.png, kb-3-nav-list-focus-windows11-fluentwinui3-dark-150.png (focus ring on Songs). Fusion and the TwIcon probe not recaptured (round-1 `evidence\` covers them; glyph set differs there, pipeline unchanged).
- Deviation: icon size 20 px (`Theme.iconSize`) where the canvas draws 18 px, per lead decision (P-0158, T-0046).
- capture.ps1: the foreground grab is occasionally flaky when two captures run back to back (a keyboard capture came out with an inactive title bar and no ring); a retry after a short pause worked; no warning printed in that case.

docs updated: design-system.md §3.4 (tint and gap wording)
deviations from ticket: none beyond round 1.

proposals:
- P: `capture.ps1` verifies the window is foreground and retries or fails loudly instead of capturing | why: an inactive window silently loses the focus ring | size: S | evidence: round-2 kb-3 first capture
- (Round-1 proposals stand: nav-list focus ring, TwIcon probe test, README layout rows.)

questions for lead: none

---
Lead (2026-11-03): looked at the light screenshot — glyphs match the canvas, crisp, aligned. P-0163 (capture.ps1
foreground verify/retry) merged → T-0050 (its acceptance depends on reliable keyboard captures; tools/capture.ps1 added
to its scope).
