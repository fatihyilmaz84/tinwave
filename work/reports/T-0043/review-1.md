REVIEW T-0043 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer, worktree @ 23511cd): fmt pass (exit 0); clippy pass (`-D warnings`, exit 0); tests pass (nextest: 495 run, 495 passed, 3 slow, 6 skipped; incl. startup `missing_root_qml_exits_nonzero...` (exit 2), `window_shell_loads_without_qml_errors`, both image_provider tests); deny pass. `git merge-tree --write-tree main t/T-0043-icon-pipeline` clean (73a4162); since the branch point main changed only nextest.toml, storage_growth tests, testing.md and work/.

acceptance criteria:
- [x] Screenshots light + dark at 150 %, tinted per token, no blur — looked at shell-*-150.png, kb-1-*-dark-150.png, zoom-4x-*, twicon-probe-*: sharp, AA edges only; selected row brighter; focus ring on the menu button. Glyph shapes don't match the design reference (blocking 1).
- [x] Token grep clean — only the existing `spacing: 0` in Main.qml:42; TwIcon/TwIcons use only Theme tokens.
- [x] Unused icons not shipped — `check_icon_usage()` (build.rs:101-111) runs before the builder and asserts both directions; 13 SVGs = 13 literal names in Sidebar.qml. The rebuild defect (finding 2) is separate.
- [x] Lucide licence — fetched `LICENSE` at tag 1.67.0: identical to icons/LICENSE-lucide.txt incl. Feather MIT notice; compiled into the qrc (build.rs:21); icons/VERSION = 1.67.0. Diffed 6 SVGs against the tag after the 2 → 1.75 stroke substitution: identical. No extra licence obligations. Top-level THIRD-PARTY-NOTICES deferred to T-0042 via icons/README.md (matches the ticket).
- [x] fmt, clippy, tests, deny pass.
- [x] P-0120 / P-0131 — tools/capture.ps1 crops to DWMWA_EXTENDED_FRAME_BOUNDS (attr 9), trims black edges, brings the window to the foreground (AttachThreadInput attached/detached around one SetForegroundWindow; one synthetic Alt press/release); env vars only for the child; `--color-scheme`; kills the child in `finally`. No registry/theme/accent/scaling changes.

blocking findings:
1. apps/tinwave-desktop/qml/Sidebar.qml:74-85 — nav glyphs don't match the design reference (canvas docs/design/canvas/Library-Windows.dc.html, `const I` lines 124-137, nav 143-146). Checked against the Lucide 1.67.0 file list:
   - Songs: canvas has two beamed notes (`M9 18V5l11-2v13` + two heads); vendored `eighth-note` is one note. `music.svg` is 404 at 1.67.0, but `eighth-notes.json` lists `music` as a deprecated alias and `eighth-notes` has the canvas shape → use `eighth-notes`.
   - Albums: canvas circle with centre dot = `disc`; vendored `disc-album` (square sleeve).
   - Artists: canvas head + shoulders = `user-round`; vendored `mic-vocal`.
   - Genres: canvas 2x2 squares = `layout-grid`; vendored `tags`.
   - This computer: canvas "This PC" folder = `folder`; vendored `monitor`.
   - Downloads: canvas arrow onto baseline = `arrow-down-to-line`; vendored `download`.
   - Import & organize: canvas path is exactly `upload`; vendored `folder-input`.
   - Settings: canvas horizontal sliders = `sliders-horizontal`; vendored `settings` (gear).
   - Matches: menu, `radio`, `activity`. Not on the canvas: Insights (`chart-column`, free), CD ripping (`disc` collides with Albums once Albums is `disc` → `disc-3` or another distinct glyph).
   - Fix: re-vendor with `tools/vendor-icons.ps1 -Add eighth-notes,disc,user-round,layout-grid,folder,arrow-down-to-line,upload,sliders-horizontal[,disc-3]`, delete unused, update `iconName` literals, recapture.
2. apps/tinwave-desktop/build.rs:39 with build.rs:115 — the build script rewrites `$OUT_DIR/icons.qrc` every run and `.qrc()` emits `cargo::rerun-if-changed` for it (cxx-qt-build 0.10 lib.rs:546), so tinwave-desktop is never fresh. Two back-to-back unchanged `cargo build -p tinwave-desktop` both recompiled (19.8 s, 14.6 s); fingerprint log: `stale: changed "...\out\icons.qrc"`. Every build/run/test/clippy reruns rcc, qmlcachegen and the desktop compile (~15 s) and relinks every desktop test binary. Fix: write icons.qrc only when content differs, or drop it and use `CxxQtBuilder::qrc_resources(...)` (`QResources`/`QResource` are re-exported by cxx-qt-build 0.10, lib.rs:38; auto-prefixes `/qt/qml/Tinwave/Desktop`; per-file rerun-if-changed). Verify a second unchanged build prints no "Compiling tinwave-desktop" and touching an SVG still rebuilds.

design check: findings above. Otherwise native slots used correctly (`icon.source`/`icon.color`/`icon.width/height` = `Theme.iconSize`); `Accessible.name` kept; icon-only button has a native ToolTip; TwIcon is `Accessible.ignored`; `TwIcons` registered as a singleton (build.rs:111) with `pragma Singleton`; crisp at 150 %.

non-blocking findings:
- Sidebar.qml:115 and design-system.md:163 — the canvas strokes every nav glyph in `text` (#1B1B1F, canvas line 36), not `textMuted` for resting rows; the builder wrote `textMuted` into §3.4. Match the canvas or have the lead/PO confirm.
- Sidebar.qml:116 — canvas icon-to-label gap is 12 px (`gap: 12px`, line 34) = `Theme.m`; branch uses `Theme.s` (8).
- TwIcons.qml:3 and tools/vendor-icons.ps1:7 — comments say `tests/icons.rs` enforces the set; it is `check_icon_usage` in build.rs.
- build.rs:55-85 — literal-substring scan: requires exactly `iconName: "`, counts commented-out references, can't see dynamic names (fails safe for vendored; runtime-caught by the startup test's `qrc:` assertion otherwise). Acceptable given the documented literal-only rule; a whitespace-tolerant regex that skips comments is cheap hardening.
- build.rs:27-31 — absolute paths unescaped in the qrc XML (`&`/`<` in the checkout path breaks rcc); goes away with `qrc_resources`.
- TwIcon.qml:9 — `QtQuick.Controls.impl` (IconImage) is non-public API; acceptable (ticket asked for IconImage); note for Qt upgrades.
- Canvas draws nav icons at 18 px; token is 20. Design system wins; canvas and tokens disagree.

proposals:
- P: Settle canvas vs design-system on nav-glyph tint and gap (text vs textMuted, 8 vs 12) | size: S
- P: build.rs icon scan whitespace-tolerant, skipping `//` comments | size: S

notes for lead: confirmed over the network (tag 1.67.0 + GitHub tree API) that `music.svg` is 404 but `eighth-notes.svg` carries `music` as a deprecated alias. Fix finding 2 before merge. T-0040/T-0044 behaviour intact. No processes left. Upstream files used: scratchpad\rev43\.

---
Lead (2026-11-03): round 2 = both blockers plus: match the canvas on tint (`icon.color: Theme.text` for every nav
glyph, fix §3.4 wording) and gap (`Theme.m`); CD ripping → `disc-3`; Insights keeps `chart-column`; use
`qrc_resources` (fixes escaping too); whitespace-tolerant, comment-skipping scan (P-0159 merged here); fix the two
comments. Icon size stays 20 (design system wins); the canvas's 18 px is a canvas-sync item (P-0158 → T-0046).
