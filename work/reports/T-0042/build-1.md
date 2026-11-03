HANDOFF T-0042 (round 1)
status: done
branch: t/T-0042-windows-packaging-notices   commit: b8c1a3a
summary: apps/tinwave-desktop/packaging/ has a scripted Windows package step (package.ps1) that builds only tinwave-desktop.exe and deploys it with the ADR 0008 recipe, adds the app-local MSVC runtime and the legal files, then runs a compliance check (check-bundle.ps1) and a Qt-off-PATH smoke test (smoke-test.ps1). test-packaging.ps1 proves each check fails on a deliberately broken bundle (13 cases, all as expected). Bundle: 1094 files, 60,453,051 bytes (57.7 MiB). THIRD-PARTY-NOTICES.txt is generated from the Qt kit's SBOMs and attribution pages plus cargo metadata. Nothing outside packaging/ touched; all four gates pass.

acceptance criteria:
- [x] Scripted deploy produces a self-contained folder that runs with Qt off PATH — `package.ps1` from a fresh worktree with QMAKE set exits 0. The smoke test starts the bundle with PATH reduced to Windows and QT_*/QML_* cleared: a window titled "Tinwave" appeared after 777 ms with 134 modules loaded; the 28 Qt ones (incl. qwindows.dll and the FluentWinUI3 plugin) all from the bundle folder. Process killed. Negative proof: "smoke test fails when Qt6Core.dll is missing, although Qt is on this shell PATH" passes.
- [x] THIRD-PARTY-NOTICES generated and included — `New-ThirdPartyNotices.ps1` writes 283,200 bytes: Qt 6.12.0 (51 modules/plugins), 45 third-party components inside Qt with full licence texts, 7 crates with their texts. Qt's LGPL-3.0/GPL-3.0 texts and the Qt source offer are in the bundle. `check-bundle` check `legal` passes. Case "notices for a package with Symphonia carry the MPL-2.0 statement and text" passes.
- [x] A check fails the build if a GPL-only Qt module or a static Qt is present — check-bundle.ps1 exits 1; package.ps1 turns that into exit 1 ("do not ship"). test-packaging.ps1 shows each failing with the right FAIL line: a Qt6QuickControlsTestUtilsPrivate.dll in the bundle; the string `Qt6QuickControlsTestUtils.lib` appended to the exe; `.qt_module("QuickTestUtilsPrivate")` in a build.rs; an exe with no Qt6 imports (cmd.exe as a static-Qt stand-in); image-stress.exe in the bundle; a missing vcruntime140.dll; a missing Qt6Core.dll; a missing source offer. Static-Qt detection reads the exe's PE import table, checks the kit's qconfig.pri is a shared build, and that no crate enables `link_qt_object_files`.
- [x] Note from T-0044 review — package.ps1 builds `--bin tinwave-desktop` and copies that one file, never `target/release/*.exe`; check `only-app-exe` and the image-stress.exe case enforce it.

checks: fmt pass, clippy pass, tests 495 passed / 0 failed (6 skipped), deny pass. No Rust code changed.

files changed (apps/tinwave-desktop/packaging/): package.ps1, check-bundle.ps1, smoke-test.ps1, test-packaging.ps1, New-ThirdPartyNotices.ps1, common.ps1; README.md; THIRD-PARTY-NOTICES.txt (committed copy; package.ps1 warns if it drifts); templates\SOURCE-OFFER-Qt.txt; license-texts\{Apache-2.0,MIT,MPL-2.0}.txt; qt-supplements\{xsvg.txt,README.md}

Bundle report (target\package\tinwave-desktop-windows-x64): 1094 files, 60,453,051 bytes (57.7 MiB); spike trimmed bundle was 1084 files, 55.4 MiB — the difference is 5 MSVC runtime DLLs (~1.5 MiB), legal files (~0.3 MiB) and the exe. Root 31 files 51.4 MiB; qml\ 1052 files 4.1 MiB; plugin folders 2.2 MiB.

signals added/changed: none
ui: none
docs updated: packaging/README.md only.

deviations from ticket:
- qtattributionsscanner.exe is in the kit but unusable: it scans qt_attribution.json files, which exist only in Qt source trees. Used the kit's SPDX SBOMs plus Qt's attribution pages from `C:\Qt\Docs\Qt-6.12.0\*\*-attribution-*.html` (the scanner's qdoc output with each licence text). The generator needs the Qt Documentation, Licenses and sbom folders and fails if any is missing.
- Qt publishes no attribution page for XSVG (in Qt Gui): its licence text was fetched byte for byte from the Qt source tag v6.12.0 and committed in qt-supplements/ with provenance. The generator fails if a component has neither a page nor a supplement.
- `cxx-qt` and `cxx-qt-lib` ship no licence file: they get the canonical MIT text from `license-texts/` with the repository as holder. Apache-2.0 and MPL-2.0 texts are copies of crate-shipped files.
- The Qt test-utils modules are static libraries in this kit, not DLLs, so `gpl-only` also scans sources (build.rs, Cargo.toml, Rust, C++, QML) and the exe for those names, and uses the SBOM licence data for every DLL.
- windeployqt runs with `--no-compiler-runtime` (with VCINSTALLDIR set it copies the 18 MB vc_redist installer). package.ps1 copies only the MSVC runtime DLLs the bundle imports, from VCToolsRedistDir, vswhere or `-CrtDir`; fails if not found. Check `closure` requires them in the bundle root.
- icons/ (T-0043) doesn't exist in this base: the generator appends every LICENSE*/COPYING*/NOTICE* under apps/tinwave-desktop/icons/ when present, read by path (untested branch).
- THIRD-PARTY-NOTICES.txt (283 KB generated) is committed per the lead note; delete it if not wanted (package.ps1 then only warns).
- Env changes are process-local and restored. From bash, `*>&1` expands `*`; use PowerShell or `2>&1`.

proposals:
- P: Docs pointer for packaging in apps/tinwave-desktop/README.md and docs/guides/desktop-dev-setup.md (package.ps1, the Qt Documentation/Licenses/SBOM components, a VS redistributable) | size: S
- P: cargo-about (not installed, not needed now) once symphonia, cpal etc. join the desktop closure | size: S
- P: Installer ticket bundles the Qt source offer contact (a project-side contact/URL is a PO decision) | size: S
- P: Wire test-packaging.ps1 and package.ps1 into the release/CI checklist (need the Qt kit and a desktop session; can't run under nextest) | size: S
- P: Root LICENSE files: the workspace says "MIT OR Apache-2.0" but the repo has no LICENSE-MIT/LICENSE-APACHE, and the MIT text needs a named holder; the bundle ships licenses\Apache-2.0.txt only | size: S

questions for lead:
- Who/what is the Tinwave-side contact or URL for the Qt written source offer, and who is the copyright holder for Tinwave's own MIT text?
- An unrelated `image-stress.exe` (PID 59376) from another worktree was running; left alone.

---
Lead (2026-11-03):
- Holder and source-offer contact are PO decisions → D-0007 (open; blocks a public release, not current work).
- The image-stress process was a reviewer's test run in another worktree; expected.
- Committing the generated THIRD-PARTY-NOTICES.txt: keep (reviewable diff when dependencies change).
- Proposals: P-0164 (docs pointers) → lead applies on merge; P-0165 (cargo-about) parked → when the audio crates join
  the desktop closure; P-0166 (source-offer contact) and P-0168 (root LICENSE files) escalated → D-0007; P-0167
  (release checklist) parked with the CI proposals.
