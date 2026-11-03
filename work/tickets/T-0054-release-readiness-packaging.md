---
id: T-0054
title: Release-readiness packaging checks (pdb ↔ exe, compiler_builtins notice, strict offline, licences folder)
type: chore
epic: E-01
status: backlog
priority: P2
size: S
depends_on: [T-0051, D-0007]
scope:
  - apps/tinwave-desktop/packaging/
owner:
branch:
review_round: 0
created_by: from T-0051 review-1 (P-0199..P-0201)
---

## Context
work/reports/T-0051/review-1.md non-blocking findings. Nothing ships before D-0007; these close the last gaps a first
public release would hit.

## Scope
- New-ThirdPartyNotices: refuse a pdb whose CodeView RSDS GUID/age doesn't match the shipped exe.
- compiler_builtins: verify against the pinned toolchain's rust-src whether its libm/musl-derived code is linked and,
  if so, add its MIT/musl notice (not covered by COPYRIGHT-library.html's "." entry).
- package.ps1: `RUSTUP_AUTO_INSTALL=0` (or fail fast when the pinned toolchain is missing).
- check-bundle qt-unmodified: only known legal files may live in `licenses\`; anything else fails.
- Document "recheck the std list on every toolchain bump" in packaging/README.md.

## Acceptance criteria
- [ ] Negative cases: mismatched pdb, a foreign file in `licenses\`, a missing toolchain with auto-install off.
- [ ] package.ps1 and test-packaging.ps1 exit 0; fmt, clippy, tests, deny pass.

## Signals & storage
None.

## Added from ADR 0009 (GPL-3.0, 2026-11-03)
- The package states where the exact corresponding source is: the public repository at the release tag (written in
  the package next to the GPL text; check-bundle -Release fails without it).
- "Appropriate Legal Notices" (GPL-3.0 §0/§5d) need an About view in the desktop UI: copyright holder, no warranty,
  licence, link to the notices. That is a UI ticket of its own (needs an artboard, D-0008/design gaps); T-0054 only
  checks the package side.

## Added from P-0204 (T-0026 build-1)
- The notices generator reads `LICENSE*` under `crates/*/vendor/**` for every crate in the shipped closure
  (first: Signalsmith Stretch + Linear in tinwave-playback). Must land before any desktop build links tinwave-playback.
