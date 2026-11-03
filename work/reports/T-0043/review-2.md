REVIEW T-0043 (round 2)
verdict: APPROVE
checks (run by reviewer, worktree @ bd4f333): fmt pass (exit 0); clippy pass (`-D warnings`, exit 0); tests: first full `cargo nextest run --workspace` 494 passed, 1 failed — `image_provider::cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` ("no run quit with requests still in flight (storm 4, scroll 0)"), a timing flake (non-blocking below); second full run 495 run, 495 passed, 6 skipped, incl. startup `missing_root_qml_exits_nonzero...`, `window_shell_loads_without_qml_errors`, both image_provider tests. deny pass. `git merge-tree --write-tree main t/T-0043-icon-pipeline` clean (b1bd8d5).

acceptance criteria:
- [x] Screenshots light + dark at 150 %, tinted per token, no blur: all four in evidence-r2 — every glyph sharp, `text` tint for resting and selected rows, evenly spaced; focus rings on the menu button and on Songs; glyphs match the canvas.
- [x] Token grep clean: outside Theme/ only `currentIndex: 0` (Sidebar.qml:70, an index) and the existing `spacing: 0` (Main.qml:42). Sidebar.qml:113-116 uses `Theme.iconSize`, `Theme.text`, `Theme.m`.
- [x] Unused icons not shipped (scratch `git archive` copy of bd4f333, own target dir, deleted): unused `icons/ghost.svg` fails the build ("Vendored but not used in QML (delete the file): ["ghost"]"), removing it builds; `iconName :  "uploadx"` reports `upload` unused and `uploadx` missing; commented `// iconName: "ghost1"` and `/* TwIcons.source("ghost2") */` build fine; 13 SVGs = 13 literals.
- [x] Lucide licence: `icons/LICENSE-lucide.txt` SHA-256 equals upstream LICENSE at 1.67.0 (B495047B…); in the resources (`rcc --list-mapping` on `cxxqtbuild/qrc/resources_0.qrc` → `:/qt/qml/Tinwave/Desktop/icons/LICENSE-lucide.txt`).
- [x] fmt, clippy, tests, deny pass (tests on the second run; flake below).
- [x] Blocker 1 (glyphs) fixed: all 13 SVGs byte-identical to lucide@1.67.0 after CRLF→LF and stroke 2→1.75. Each nav row matches the canvas `const I` (Library-Windows.dc.html:124-137, nav 143-146): Songs eighth-notes; Albums disc; Artists user-round; Genres layout-grid; Smart Radio radio; This computer folder; Downloads arrow-down-to-line; Import & organize upload; Signal Center activity; Settings sliders-horizontal (Lucide's 3-row version of the canvas's 2-row sliders; closest). Not on the canvas: Insights chart-column; CD ripping disc-3 (visibly distinct from Albums).
- [x] Blocker 2 (freshness) fixed: two unchanged `cargo build -p tinwave-desktop` printed only `Finished` (0.57 s, 0.30 s); clippy fresh; scratch copy same (1.73 s, 0.27 s). `touch icons/menu.svg` → `Compiling tinwave-desktop` (12.4 s), next build fresh (0.20 s). build.rs:24-29 uses `qrc_resources(QResources…)`; output shows `rerun-if-changed=icons` plus one line per file, nothing under OUT_DIR. Repeated `Compiling tinwave-desktop` when alternating `nextest --workspace` and `build -p` is feature unification (`UnitDependencyInfoChanged { cxx … }`), not build.rs.
- [x] Item 3 tint and gap: Sidebar.qml:115 `icon.color: Theme.text`, :116 `spacing: Theme.m`; design-system.md:163 matches.
- [x] Item 4 hardened scan (build.rs functions compiled into a standalone harness, scratchpad\rev43scan\scan.rs): `"http://x/y"` and `"a//b" // iconName: "ghost"` not eaten; escaped `\"` handled; `/* … */` stripped; `iconName  :   "b"` matched; `name:` inside `TwIcon { … }` found incl. nested braces; `Accessible.name`, `objectName`, `model.iconName`, `TwIcons { name: … }` and TwIcon.qml's `TwIcons.source(root.name)` not counted.
- [x] Item 5 comments: TwIcons.qml:3-4 and vendor-icons.ps1:7 point to `check_icon_usage`.
- [x] T-0040/T-0044 behaviour intact (round 2 touches no src/, cpp/, tests/ or stress scenes).

blocking findings:
- none

design check: pass. Native icon slots kept; `Accessible.name` and ToolTip on the icon-only menu button; tints and gap from tokens; light and dark right at 150 %. Canvas 18 px vs token 20 px is the accepted deviation (P-0158 → T-0046).

non-blocking findings:
- apps/tinwave-desktop/tests/image_provider.rs:196 (T-0044 test, not in this diff) — `cancel_storm…` is timing-dependent: failed 2 of 14 runs on this branch (once under full-workspace parallel load with the 90-day storage_growth tests alongside, once right after a fresh link; both "scroll 0"); passed 12, and 5/5 on main's binary. The branch doesn't touch the provider, scene or test. Worth a ticket: it will make `cargo nextest run --workspace` red at random.
- build.rs:33-60 (`strip_comments`) only knows double-quoted strings: a single-quoted/backtick literal containing `//` eats the rest of the line (loud false "unused"); `'it"s' // iconName: "x"` counts a commented-out name as used. No QML uses `'…'` or backtick strings today. ~3 lines to fix.
- build.rs:96-117 (`twicon_blocks`) — a nested `State { name: "pressed" }` inside a `TwIcon { }` is counted as an icon name (fails loudly as "missing").
- The lead's note that `qrc_resources` "fixes escaping too" is not fully right: qt-build-utils 0.10 `qrc.rs:46-58` writes the canonicalised path into `<file>` without XML-escaping; a checkout path with `&` or `<` still breaks rcc, now upstream. Low impact; don't record it as fixed.
- design-system.md:163 — the edited bullet isn't wrapped like its neighbours. Cosmetic.

proposals:
- P: Make the image_provider cancel-storm test robust to load (or bounded retry / heavy in nextest.toml) | size: S
- P: icon scan treats `'…'` and backtick strings as strings | size: S

notes for lead: merge clean. Reproductions on a scratch copy (deleted; C: and D: ~98–99 % full). Upstream SVGs in scratchpad\rev43r2up\. Worktree untouched. No processes left. A red `cancel_storm…` in a gate run is not caused by this branch — rerun.

---
Lead (2026-11-03): merge. Correction recorded: `qrc_resources` removes our unescaped XML but qt-build-utils still
doesn't escape paths (upstream; low impact). P-0169 (cancel-storm flake) merged → T-0049, raised to P1 (gate flake).
P-0170 (single-quote/backtick strings in the scan) parked — revisit when QML using those strings references icons.
Disk: the lead removed 14.4 GB of build output from the merged T-0040/T-0024 leftover folders (D: 10 → 23.5 GB free).
