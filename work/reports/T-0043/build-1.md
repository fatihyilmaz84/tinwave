HANDOFF T-0043 (round 1)
status: done
branch: t/T-0043-icon-pipeline   commit: 23511cd
summary: 13 Lucide 1.67.0 icons vendored at stroke 1.75 and compiled into the QML module resources. Licence text kept; build.rs fails the build if the vendored set and the icon names used in QML differ. The app-menu button and all 12 sidebar nav items show tinted glyphs through the native `icon.source`/`icon.color` slots. `TwIcon` and a `TwIcons` URL singleton exist (TwIcon not used by the app yet). Two dev-only PowerShell tools: a Lucide vendoring script and the screenshot capture helper (P-0120 / P-0131).

acceptance criteria:
- [x] Screenshots light + dark at 150 %, icons tinted per token, no blur. Pixel peaks inside glyphs equal the tokens: menu and selected row = text (242,242,244), resting row = textMuted (166,166,175) in dark. 4x nearest-neighbour zoom shows only anti-aliased edges.
- [x] Token grep clean; no raw sizes/colours: `#hex` / `pixelSize: <n>` grep over qml/ outside Theme/ empty; sizes from `Theme.iconSize`, `Theme.s`, `Theme.sidebarRowHeight`; only bare number is the existing `spacing: 0` in Main.qml.
- [x] Unused icons not shipped: `check_icon_usage()` in build.rs runs every build. Both failure modes verified by hand: stray `zzz-unused.svg` → "Vendored but not used in QML (delete the file): ["zzz-unused"]"; `TwIcons.source("menux")` → "Used in QML but not vendored: ["menux"]". Reverted.
- [x] Lucide licence in the notices source: `apps/tinwave-desktop/icons/LICENSE-lucide.txt` (verbatim ISC from the pinned tag, incl. Lucide's Feather MIT notice), also compiled into resources at `qrc:/qt/qml/Tinwave/Desktop/icons/LICENSE-lucide.txt`. `icons/README.md` tells T-0042 to include it verbatim in THIRD-PARTY-NOTICES. No top-level THIRD-PARTY-NOTICES created (T-0042 builds it).
- [x] fmt, clippy, tests, deny pass.
- [x] P-0120: `tools/capture.ps1` crops to DWMWA_EXTENDED_FRAME_BOUNDS, then trims fully-black edge lines (DWM's own 1 px outline at 150 % left a 2 px strip); final PNGs have no black edge pixels.
- [x] P-0131: the helper brings the window to the foreground before sending keys (Alt tap, AttachThreadInput, SetForegroundWindow; warns if that fails). Keyboard captures show an active title bar and focus rings (menu button in `kb-1-*`, nav row in `kb-3-*`). Uses `--color-scheme`; never touches the OS theme or accent.

checks: fmt pass, clippy pass, tests 495 passed / 0 failed (6 skipped), deny pass

files changed (apps/tinwave-desktop/ unless noted): build.rs; qml/Sidebar.qml; qml/Tw/TwIcon.qml (new); qml/Tw/TwIcons.qml (new); icons/*.svg (13 new); icons/VERSION, icons/LICENSE-lucide.txt, icons/README.md (new); tools/vendor-icons.ps1 (new); tools/capture.ps1 (new); docs/design/design-system.md (repo root)

signals added/changed: none

ui:
- Design reference: design-system §3.4, §4, §5. The canvas artboard was not visible to the builder; glyph choices are the builder's.
- Native `ToolButton` (icon only, `Accessible.name` "App menu", native `ToolTip` "App menu"). Native `ItemDelegate` with `icon.source`, `icon.width/height` = `Theme.iconSize`, `icon.color` = `Theme.text` selected / `Theme.textMuted` resting, `spacing: Theme.s`. `TwIcon` unused in the app (every control had a native slot); rendered through a throwaway probe (`twicon-probe-dark-150.png`).
- Glyphs: menu, eighth-note (Songs), disc-album (Albums), mic-vocal (Artists), tags (Genres), radio (Smart Radio), chart-column (Insights), monitor (This computer), download (Downloads), folder-input (Import and organize), disc (CD ripping), activity (Signal Center), settings (Settings). Lucide 1.67 has no `music` icon (per the builder).
- OS checked: Windows 11, FluentWinUI3 at 150 % light and dark; Fusion light. Not done: non-default OS accent; real 100 %/200 % render.
- Screenshots (not committed; outside scope): scratchpad\t0043\evidence\ — shell-windows11-fluentwinui3-light-150.png, shell-windows11-fluentwinui3-dark-150.png, shell-windows11-fusion-light-150.png, kb-1-menu-button-focus-windows11-fluentwinui3-{light,dark}-150.png, kb-3-nav-list-focus-windows11-fluentwinui3-light-150.png, zoom-4x-nearest-sidebar-dark-150.png, twicon-probe-dark-150.png.
- Theme.qml untouched (`iconSize`, `iconSizeDense`, `iconStroke` already existed).
- Observations: Fusion rows render as white blocks with solid accent selection (stock Fusion). Pre-existing from T-0040: after Up/Down in the nav list the focus ring disappears (it stays on the first delegate while the selection moves).

docs updated: design-system.md §3.4 "How it is built" block (vendored set, build check, baked stroke, native-slot rule, notices source); §5 `TwIcon` row; icons/README.md new.

deviations from ticket:
- Native `icon.*` slots for the menu button and nav items instead of TwIcon (the ticket's UI section rule); TwIcon has no in-app use yet.
- `TwIcons.qml`: new `pragma Singleton` helper (`source(name)`), needed because native controls need a URL and `Qt.resolvedUrl` is location-specific.
- Build check in build.rs, not a test (tests/ not in scope).
- Icon resources as a generated `$OUT_DIR/icons.qrc` passed with `.qrc()` (`QResourceFile` not re-exported by cxx-qt-build; the alternative was a new build-dependency on qt-build-utils). No new Cargo dependency.
- build.rs `#![allow(clippy::expect_used)]` with a comment (build scripts report failures by panicking).
- Stroke 1.75 baked into the SVGs by the vendoring script; `Theme.iconStroke` documents the value.
- Native `ToolTip` on the menu button (an icon-only control needs its name visible).

proposals:
- P: Fix the nav list focus ring after Up/Down | why: keyboard users lose the ring (design-system §7) | size: S
- P: Move focus onto the current item, or draw the ring from `ListView.isCurrentItem` + `activeFocus` in Sidebar.qml | why: likely fix for the above | size: S
- P: tests/icons.rs (headless) loading a TwIcon probe through `TINWAVE_MAIN_QML`, asserting no `qrc:` warnings | why: TwIcon has no automated coverage | size: S
- P: README layout table rows for `icons/`, `tools/capture.ps1`, `tools/vendor-icons.ps1`, `qml/Tw/` | size: S
- P: Accent-override dev switch (already proposed in T-0035) | size: S

questions for lead:
- Keep `TwIcons.qml` (singleton URL helper), or fold it elsewhere (e.g. Theme)?
- Is the glyph mapping acceptable (eighth-note for Songs, chart-column for Insights, disc for CD ripping)? The canvas was not visible.

---
Lead (2026-11-03):
- Keep `TwIcons` as its own singleton: it is a resource lookup, not a token, so it doesn't belong in Theme.
- Glyph mapping: the canvas defines its nav icons as SVG paths (docs/design/canvas/Library-Windows.dc.html, `const I`
  at line 124; nav at 143-146). The reviewer compares each glyph with the canvas shape; mismatches are a review finding.
- Proposals: P-0154 (nav focus ring after Up/Down; P-0155 merged into it) accepted → T-0050; P-0156 (TwIcon headless
  test) merged → T-0045; P-0157 (README layout rows) → lead applies on merge; the accent switch is P-0132 (→ T-0036).
- Screenshots: the lead copies the evidence PNGs into apps/tinwave-desktop/docs/screenshots on merge.
