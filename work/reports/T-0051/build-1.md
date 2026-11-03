HANDOFF T-0051 (round 1)
status: done
branch: t/T-0051-packaging-hardening   commit: f25e5f0
summary: Every T-0042 review finding and P-0171..P-0174 closed inside apps/tinwave-desktop/packaging/ plus the About-dialog rule in docs/rules/qml-ui.md. check-bundle gained link-libs, qt-unmodified, source-offer and d0007 checks. The notices carry the std crates, the KDAB holder and the FTL/IJG sentences. D-0007 is a "pending" marker that does not fail the package. package.ps1 and test-packaging.ps1 both exit 0 (31 cases, each new check has a negative case); Rust gates pass.

acceptance criteria:
- [x] package.ps1 and test-packaging.ps1 exit 0, new negative cases for each new check — package.ps1 exit 0 (smoke: window 'Tinwave' after 482 ms, 28 Qt modules all from the bundle). test-packaging "31 cases, 0 failed" (was 13). Negative cases: missing repo in the offer (qtsvg line removed → `FAIL source-offer .*qtsvg`; pruned `official_releases` link also fails); modified Qt file (Qt6Svg.dll +1 byte, and a foreign extra.qml → `FAIL qt-unmodified`); GPL-only lib visible only via rustc-link-lib (fixture build dir with `cargo::rustc-link-lib=Qt6QuickTestUtils`, exe untouched → `FAIL link-libs ... GPL-only`; `static=Qt6Core` and an empty build dir also fail); plus FTL/IJG sentence removed, -Release with D-0007 pending, stale bundle after D-0007 is filled in, pdb naming no std rlib, -OutDir without marker, smoke asserts `process exited with code -1073741515`, qwindows maps to QWindowsIntegrationPlugin only.
- [x] Regenerated THIRD-PARTY-NOTICES committed — byte-identical to the generated one (cmp, no drift warning); case "desktop notices: KDAB holder..., std crates from the link, FreeType and IJG sentences, D-0007 marker" passes; Lucide section included.
- [x] fmt, clippy, tests, deny. No Rust changed.

checks: fmt pass, clippy pass, tests 499 passed / 0 failed (6 skipped), deny pass
files changed (apps/tinwave-desktop/packaging/ unless noted): common.ps1, check-bundle.ps1, package.ps1, smoke-test.ps1, New-ThirdPartyNotices.ps1, test-packaging.ps1, README.md, THIRD-PARTY-NOTICES.txt, templates/SOURCE-OFFER-Qt.txt, qt-supplements/README.md, release-identity.json (new); docs/rules/qml-ui.md (About dialog section only)
signals added/changed: none
ui: none
docs updated: packaging/README.md (checks table, D-0007 section, std/FreeType/IJG sources, -Release, marker, frozen/offline), qt-supplements/README.md (XSVG provenance), docs/rules/qml-ui.md (About dialog, LGPL-3 §4(c))

What each finding became:
- Source offer: template has `{{SOURCE_LIST}}`, generated per shipped Qt repo from the SBOM file names (qtbase, qtdeclarative, qtsvg) with `download.qt.io/archive/qt/6.12/6.12.0/submodules/...` as primary links. check-bundle `source-offer` fails on a missing repo, a missing archive link, `official_releases`, or an unfilled placeholder. Template's "Tinwave 6.12.0-based releases" wording fixed.
- FTL and IJG: emitted when those components ship; the year is the newest copyright year on the FreeType attribution pages (2026 for 2.14.3). check-bundle `legal` fails without them.
- cxx-qt and cxx-qt-lib MIT holder: "2021-2025 / 2021-2026 Klarälvdalens Datakonsult AB, a KDAB Group company <info@kdab.com>" from the crates' SPDX-FileCopyrightText; fallback Cargo authors; with neither the generator fails.
- Rust std from the link: target\release\tinwave_desktop.pdb names the rlibs the linker took; those with the same hash in the pinned toolchain's sysroot are std's: alloc, compiler_builtins, core, panic_unwind, std (in-tree), hashbrown 0.15.5 and rustc-demangle 0.1.26 (vendored). Texts from the toolchain's `share\doc\rust\COPYRIGHT-library.html` and `share\doc\rust\licenses\*.txt`; in-tree entries listed (unicode_data.rs Unicode-3.0, backtrace, mpmc, fuchsia). gimli, addr2line, miniz_oxide are not linked on x86_64-pc-windows-msvc (dbghelp backtraces) → "contribute no code".
- XSVG: no Qt 6.12.0 source offline; xsvg.txt unchanged (first line has two spaces — a lost ©). qt-supplements/README.md states this, gives the SHA-256, says the SBOM line "Copyright 2002 USC/Information Sciences Institute" is authoritative (printed directly above the text), and gives the refresh procedure; the "byte for byte" claim removed.
- check-bundle `link-libs` reads target\release\build\*\output rustc-link-lib lines and looks each Qt lib up in the SBOM (Core, Gui, Qml, Quick, QuickControls2); fails on GPL-only, static=, no SBOM entry, or no outputs. `qt-unmodified` compares SHA-256 of 1084 Qt files against the kit; fails on any file neither the kit's nor ours.
- test-packaging asserts the -1073741515 reason; smoke-test saves and restores SetErrorMode in finally.
- -OutDir deleted only if empty or carrying `.tinwave-package` (written by package.ps1; also in the bundle). A pre-change bundle lacks the marker: the first run needs the old folder removed by hand.
- `--frozen` for cargo build; `--locked --offline` for both cargo metadata calls. Plugin match exact (alias table entry for qwindows): Direct2D no longer listed (51 → 50 modules); prefix-only names fail closed.
- D-0007 (nothing invented): `release-identity.json` null/null. Pending: notices say `Copyright holder: [D-0007 PENDING: copyright holder not decided]`, the offer `Contact for this offer: [D-0007 PENDING: source-offer contact not decided]`; check-bundle prints `PENDING d0007 …` listing holder, contact, root LICENSE-MIT / LICENSE-APACHE, still exit 0. Flip: fill the two JSON values, add the two root LICENSE files (shipped as licenses\Tinwave-LICENSE-*.txt), build with `package.ps1 -Release` — `-Release` turns any pending item into FAIL and a bundle still carrying the marker is "stale" and fails. A test proves the flipped path.

deviations from ticket:
- The source-file name scan (build.rs/Cargo.toml/QML tokens, -SourceDir) removed, replaced by link-libs as the ticket says; the exe-string scan stays as a backstop.
- test-packaging deletes its fixtures (~940 MB) after a clean run; kept on failure.
- New file release-identity.json and params `-Release`, `-BuildDir`, `-Identity` (check-bundle), `-Pdb`, `-IdentityFile` (notices). The notices generator needs target\release\tinwave_desktop.pdb (release build first); fails clearly otherwise.

proposals:
- P: Wire package.ps1 -Release and test-packaging.ps1 into the release checklist/CI | size: S
- P: Round-trip the XSVG supplement from a Qt 6.12.0 source tarball when available | size: S
- P: Installer ticket bundles `licenses\` and copies by manifest, honouring `.tinwave-package` | size: S

questions for lead: none. Long-running python3 (1-2 Nov) and one python (2 Nov) processes left alone (not the builder's). All tinwave-desktop processes gone.

---
Lead (2026-11-03): P-0191 merged → P-0167 (parked with CI); P-0192 (XSVG round-trip) parked → before first release;
P-0193 parked → installer ticket.
