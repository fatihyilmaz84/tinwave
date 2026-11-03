---
id: T-0051
title: Packaging hardening and notices completeness (before any public release)
type: chore
epic: E-01
status: in_review
priority: P1
size: M
depends_on: [T-0042]
scope:
  - apps/tinwave-desktop/packaging/
  - docs/rules/qml-ui.md            # the About-dialog LGPL rule only (P-0174)
owner: tw-builder
branch: t/T-0051-packaging-hardening
review_round: 1
created_by: from T-0042 review-1 (non-blocking findings, P-0171..P-0174)
---

## Context
work/reports/T-0042/review-1.md lists the findings in full. Nothing ships before D-0007 (holder, source-offer contact)
is decided; this ticket closes the remaining compliance and robustness gaps so a release is mechanical afterwards.

## Scope
Notices and offer:
- Source-offer repo list generated from the SBOM repos of the shipped packages (or check-bundle fails on a missing
  repo); `download.qt.io/archive/qt/<maj.min>/<ver>/submodules/...` as the primary link.
- Explicit FreeType (FTL) and IJG acknowledgement sentences in THIRD-PARTY-NOTICES.
- cxx-qt / cxx-qt-lib MIT holder from their SPDX-FileCopyrightText (KDAB), not "the cxx-qt authors".
- Rust std and the std-vendored crates (hashbrown, gimli, addr2line, miniz_oxide, …) listed with their texts.
- Re-copy qt-supplements/xsvg.txt from a Qt 6.12.0 source (the "©" likely dropped); fix the README claim. If no
  source checkout is available offline, say so and leave a TODO-free note on provenance instead.
Checks and scripts:
- check-bundle reads `target/release/build/*/output` `rustc-link-lib` lines and looks each Qt lib up in the SBOM
  (replaces name heuristics for static GPL-only libs); every shipped Qt file byte-identical to the kit.
- test-packaging asserts the smoke failure reason (-1073741515), not just "FAIL smoke".
- smoke-test restores `SetErrorMode`.
- `-OutDir` deleted only when it carries the script's own marker file.
- `--frozen` for cargo build and `--offline` for cargo metadata at package time.
- common.ps1 plugin-name match exact (no qwindows → Direct2D over-inclusion).
Rule:
- qml-ui.md: the future About dialog shows the Qt (LGPL) notice and points to the licence texts (LGPL-3 §4(c)).

## Acceptance criteria
- [ ] package.ps1 and test-packaging.ps1 exit 0; new negative cases for each new check (missing repo in the offer,
      a modified Qt file, a GPL-only lib only visible via rustc-link-lib).
- [ ] Regenerated THIRD-PARTY-NOTICES committed (std crates, KDAB holder, FTL/IJG sentences).
- [ ] fmt, clippy, tests, deny pass.

## Signals & storage
None.
