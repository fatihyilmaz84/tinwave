REVIEW T-0042 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy pass, tests pass on rerun: nextest 495/495, 6 skipped. First full run had 1 failure, `tinwave-desktop::image_provider cancel_storm_with_requests_in_flight_then_quit_exits_cleanly` (14.1 s); it passed alone (9.4 s) and in a second full run (15.3 s); the branch changes no Rust code. deny pass (deny.toml unchanged; MPL still scoped to symphonia*).
merge-tree: `git merge-tree --write-tree main t/T-0042-windows-packaging-notices` clean (tree 7236312) against main ccb30db (incl. T-0043). Merge base 9c36f0f.

Packaging runs (worktree, Qt env):
- `package.ps1` exit 0. Bundle target\package\tinwave-desktop-windows-x64: 1094 files, 60,453,051 bytes (57.7 MiB). check-bundle 7 PASS. Smoke: "window 'Tinwave' visible after 630 ms; 134 modules loaded, 28 of them Qt, all Qt files from the bundle: True". No drift warning (committed notices SHA-256 CC15E181…F0BB = generated).
- `test-packaging.ps1` exit 0: 13 cases, 0 failed.
- No tinwave-desktop process left; everything produced is under target\.

acceptance criteria:
- [x] Self-contained folder runs with Qt off PATH: package.ps1 exit 0 and smoke PASS; smoke-test.ps1:38-40 clears QT_*/QML_* and sets the child's PATH to System32 and Windows only. Ran smoke-test.ps1 on the `smoke-no-core` fixture with Qt prepended to my PATH: exit -1073741515 (STATUS_DLL_NOT_FOUND) — the PATH reduction works. All 1084 Qt files in the bundle are byte-identical to the kit ("unmodified Qt" holds). Every delay-load import resolves to System32 or an api-set.
- [x] THIRD-PARTY-NOTICES generated and included: Qt attribution pages + SBOMs + cargo metadata (acceptable deviation from qtattributionsscanner). The 45 components match every `<host>_Attribution_*` entry in the kit SBOMs whose host ships (Core 15, Gui 16, Network 3, Qml masm, QuickLayouts yoga, QWindows wintab, bundled libs). Qt spot-check: FreeType 2.14.3 (FTL), HarfBuzz-NG 14.5.0, libjpeg-turbo 3.2.0 (IJG), libpng 1.6.58, PSL data (MPL-2.0). Crate spot-check: cxx (LICENSE-APACHE/MIT), foldhash (Zlib), static_assertions; the 7 crates match `cargo tree -p tinwave-desktop -e normal`. Qt LGPL-3/GPL-3 texts and SOURCE-OFFER-Qt.txt in the bundle. MPL case passes (15 symphonia crates).
- [x] GPL-only / static-Qt check: test-packaging cases 2–5 pass. check-bundle.ps1:39-54 reads qconfig.pri (shared on, static off) and link_qt_object_files from cargo metadata; :57-67 PE imports; :75-107 SBOM lookup per DLL (fails closed with no SBOM entry) plus source/exe name scan. Across all kit SBOMs the only non-tool GPL-only packages are QuickTestUtilsPrivate and QuickControlsTestUtilsPrivate; all 51 shipped packages are "Commercial OR LGPL-3.0-only OR GPL-2/3". Build-script outputs show only Qt6Core/Gui/Qml/Quick/QuickControls2 import libs plus cxx-qt's generated static libs. Nothing GPL-only missed.
- [x] T-0044 note: package.ps1:49 builds `--bin tinwave-desktop`, :62 copies one file; it's the only .exe in the bundle; the image-stress case passes.

blocking findings:
- none

design check: n/a

non-blocking findings:
- templates/SOURCE-OFFER-Qt.txt:12-20 hard-codes qtbase, qtdeclarative, qtsvg (correct today); a module from another repo wouldn't be noticed. Generate the list from the shipped packages' SBOM repos, or fail check-bundle on a repo not in the template.
- SOURCE-OFFER-Qt.txt:9-10: `official_releases` is pruned; make `download.qt.io/archive/qt/<maj.min>/<ver>/submodules/...` the primary link.
- qt-supplements/xsvg.txt:1 "Copyright  2002" (two spaces; SBOM "Copyright 2002") — likely a dropped "©"; the README's "byte for byte" claim is probably wrong. Re-copy from a Qt 6.12.0 source checkout.
- New-ThirdPartyNotices.ps1:178-179: for cxx-qt and cxx-qt-lib the MIT holder is "the cxx-qt authors (repo URL)"; their sources carry `SPDX-FileCopyrightText: Klarälvdalens Datakonsult AB, a KDAB Group company` — use that.
- No explicit acknowledgement sentences: FTL and IJG appear only as licence text (incl. the FTL "<year>" template). Add "Portions of this software are copyright © 2026 The FreeType Project (www.freetype.org). All rights reserved." and "This software is based in part on the work of the Independent JPEG Group."
- smoke-test.ps1:27: `SetErrorMode` runs in package.ps1's own process and is never restored (an interactive shell keeps it).
- test-packaging.ps1:115 asserts only "FAIL smoke"; assert the -1073741515 exit reason.
- smoke-test.ps1:62: Qt modules recognised by name prefix or `\Qt\` path; a plugin from elsewhere with an unmatched name wouldn't be flagged (low risk with QT_PLUGIN_PATH cleared).
- check-bundle.ps1:88-105: static GPL-only detection relies on name scans of apps/tinwave-desktop and exe strings; a `rustc-link-lib` from another crate's build script is caught only by the string heuristic.
- common.ps1:168: prefix match maps qwindows.dll to QWindowsDirect2DIntegrationPlugin too (harmless over-inclusion).
- package.ps1:54-59: `-OutDir` is deleted whenever it contains a tinwave-desktop.exe — footgun; delete only a folder carrying the script's own marker.
- package.ps1:49: `--locked` allows downloads on a cold cache; use `--frozen` (and `--offline` for cargo metadata) if "no network at package time" must hold strictly.

proposals:
- P: check-bundle reads build-script outputs (`rustc-link-lib`) and looks up each linked Qt lib in the SBOM | size: S
- P: Source-offer repo list from the SBOM; check every shipped Qt file is byte-identical to the kit | size: S
- P: Rust std and vendored std crates (hashbrown, gimli, addr2line, miniz_oxide, …) in THIRD-PARTY-NOTICES (std is statically linked, MIT OR Apache-2.0) | size: S
- P: LGPL-3 §4(c) rule for the future About dialog (show the Qt notice, point to the licence texts) | size: S

notes for lead: main moved under the branch (T-0043 with icons/LICENSE-lucide.txt). Merge is textually clean, but the committed THIRD-PARTY-NOTICES.txt is then stale (no Lucide section) and the generator's icons branch has never run: have the branch merge main, re-run package.ps1 + test-packaging.ps1, commit the regenerated notices. Flaky cancel_storm test noted. D-0007 items are the only gaps before a public release.

---
Lead (2026-11-03): approve stands. Before merge: the builder merges main into the branch, re-runs package.ps1 and
test-packaging.ps1, and commits the regenerated notices (Lucide section). All non-blocking findings and the four
proposals → T-0051 "packaging hardening + notices completeness" (P-0171..P-0174), P1, must land before any public
release (with D-0007). cancel_storm flake is T-0049.
