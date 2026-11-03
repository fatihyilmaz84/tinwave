REVIEW T-0051 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy pass (-D warnings), tests pass (nextest: 499 run, 499 passed, 6 skipped, 34.9 s), deny pass
merge-tree: `git merge-tree --write-tree main t/T-0051-packaging-hardening` clean (tree fd5c445). Main f46a077, merge base 2d6ee4b (main only gained board/work commits). Single commit f25e5f0; nothing in `work/`, no TODO/FIXME, no Rust changes.

Packaging runs (worktree, Qt env; logs in the session scratchpad):
- `package.ps1` exit 0 in 20 s (`cargo build --release --frozen` no-op). Source offer: 3 repos (qtbase, qtdeclarative, qtsvg). Notices: 50 Qt modules/plugins, 45 components, 7 crates, 1 asset licence. check-bundle passed all 10 checks: shared-qt x2, static-qt, gpl-only, link-libs (5 libs), qt-unmodified (1084 files), only-app-exe, closure (56 PE files), legal, source-offer; d0007 PENDING (holder, contact, LICENSE-MIT, LICENSE-APACHE). Smoke: "window 'Tinwave' visible after 617 ms; 134 modules, 28 Qt, all from the bundle". Bundle 1095 files, 60,538,270 bytes (57.7 MiB). Generated notices SHA-256 = committed (F37CD4E5…C8F2), no drift.
- `test-packaging.ps1` exit 0 in 197 s, "31 cases, 0 failed"; fixtures removed.
- The worktree bundle already had `.tinwave-package`; case 11d covers the pre-change situation (a folder with tinwave-desktop.exe but no marker is refused with "does not carry .tinwave-package", exe still there).
- No processes left; nothing written outside target\ and %TEMP% (smoke-test temp files, deleted); `git status` clean.

acceptance criteria:
- [x] package.ps1 and test-packaging.ps1 exit 0, each new check with a negative case failing for its reason: missing repo → `FAIL source-offer .*qtsvg` (pruned `official_releases` also fails); Qt6Svg.dll +1 byte → `FAIL qt-unmodified .*Qt6Svg.dll`, foreign extra.qml "not a file of the Qt kit"; GPL-only lib via rustc-link-lib only → `FAIL link-libs .*Qt6QuickTestUtils .*GPL-only` (maps via `Find-QtLibSbomPackage` to QuickTestUtilsPrivate); `static=Qt6Core` and an empty build dir fail; missing FTL/IJG → `FAIL legal`; `-Release` with D-0007 pending → `FAIL d0007`; stale bundle after identity filled fails; fake pdb refused; smoke asserts `process exited with code -1073741515`; exact plugin match (qwindows → QWindowsIntegrationPlugin only; qwin/qwindowsx match nothing).
- [x] Regenerated THIRD-PARTY-NOTICES committed: lines 59-60 FTL (2026) and IJG sentences; lines 4983 and 5010 "Klarälvdalens Datakonsult AB, a KDAB Group company <info@kdab.com>" (cross-checked against SPDX-FileCopyrightText in cxx-qt-0.10.0 (2021-2025) and cxx-qt-lib-0.10.0 (2021-2026); "the cxx-qt authors" gone); §6 "Rust standard library" (line 4719) in-tree list, hashbrown 0.15.5, rustc-demangle 0.1.26, texts in §7; Lucide §8; line 14 D-0007 marker.
- [x] fmt, clippy, tests, deny pass.

T-0042 findings checked one by one:
- Source offer from the SBOM repos of the shipped DLLs; links `download.qt.io/archive/qt/6.12/6.12.0/submodules/<repo>-everywhere-src-6.12.0.tar.xz`; check-bundle fails on a missing repo, missing archive link, `official_releases`, or a `{{` placeholder.
- std crates from the pdb (re-scanned): sysroot rlibs named = alloc, compiler_builtins, core, hashbrown, panic_unwind, rustc_demangle, std (+ 4 Cargo rlibs) — matches the notices. Not named: cfg_if, windows_targets (macro-only), getopts, rustc_literal_escaper (test/proc_macro), std_detect, unwind, sysroot, test, proc_macro, profiler_builtins, panic_abort, rustc_std_workspace_* (in-tree, covered by "."). Correct for 1.92.0 on x86_64-pc-windows-msvc; texts from COPYRIGHT-library.html and licenses\*.txt.
- XSVG provenance honest: no Src in C:\Qt\6.12.0; SBOM `Gui_Attribution_xsvg` copyrightText matches the README; file hash matches (3A9465…A9BD, 1185 B); notices print the SBOM line above the text (4619/4621).
- SetErrorMode saved (smoke-test.ps1:29), restored in finally (:75).
- `-OutDir` deleted only if empty or carrying `.tinwave-package`; marker written each time.
- No network: `--frozen`; both `cargo metadata` calls `--locked --offline`.
- Exact plugin match via alias table + exact / minus-"plugin" tiers; unmatched fails closed; Direct2D no longer included.
- link-libs reads the real outputs: only Qt6Core, Gui, Qml, Quick, QuickControls2 dynamic; `static` entries are cxx-qt's generated libs, cxxbridge1, link-cplusplus (none Qt); no `rustc-link-arg`.

D-0007: nothing invented (release-identity.json null/null; root LICENSE files absent); marker in the shipped notices and the offer; `-Release` passes through to check-bundle and turns pending into FAIL; flip path tested (11a) — only the root LICENSE files stay pending; their final step can't be tested without writing at the repo root (acceptable).

blocking findings:
- none

design check: n/a (no UI; the qml-ui.md About-dialog rule correctly states LGPL-3 §4(c))

non-blocking findings:
- New-ThirdPartyNotices.ps1:239 — the pdb isn't tied to the shipped exe (no RSDS GUID/age compare); with `-SkipBuild` after a different build on the same toolchain the std list could describe another link (a different toolchain's pdb fails closed).
- New-ThirdPartyNotices.ps1:236-246 — the pdb module list shows only rlibs that contributed objects; a vendored std crate whose code is only inlined/monomorphised elsewhere would be missed. Correct today; recheck on every toolchain bump.
- compiler_builtins is linked (`compiler_builtins::math::libm_math::support` symbols); Rust's COPYRIGHT-library.html covers it only via "." (Apache-2.0 OR MIT); compiler-builtins' libm portions are MIT with a musl notice. Unverified offline (no rust-src); worth a look before the first release.
- Rustup proxies can auto-install a missing pinned toolchain (network) despite `--frozen`; `$env:RUSTUP_AUTO_INSTALL='0'` in package.ps1 makes "no network" strict.
- check-bundle.ps1:140 — qt-unmodified skips all of `licenses\*`; a non-legal file there escapes the kit comparison (DLLs would still hit gpl-only and closure).
- packaging/README.md:34 — reference run says "1094 files"; with the marker it is 1095.

proposals:
- P: Check tinwave_desktop.pdb belongs to the shipped exe (CodeView RSDS GUID/age) before deriving the std list | size: S
- P: Before the first release, verify compiler_builtins' libm/musl MIT notice against rust-src and add it if linked | size: S
- P: RUSTUP_AUTO_INSTALL=0 (or fail if the pinned toolchain is missing) in package.ps1 | size: S

notes for lead: merge clean. Another agent's tinwave-playback stress run and old python processes were on the machine; runs passed anyway. D-0007 is the only gate before a public release; `package.ps1 -Release` enforces it.

---
Lead (2026-11-03): merge. README "1094" fixed by the lead on main. The remaining non-blocking items and P-0199..P-0201
→ T-0054 (release-readiness packaging checks, P2, before the first release with D-0007).
