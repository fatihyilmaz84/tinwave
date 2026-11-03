---
id: T-0058
title: Switch Tinwave's own licence to GPL-3.0-only in Cargo, cargo-deny and packaging
type: chore
epic: E-01
status: in_progress
priority: P1
size: M
depends_on: [D-0009]
scope:
  - Cargo.toml          # [workspace.package] license line only
  - deny.toml           # [licenses.private] only
  - apps/tinwave-desktop/packaging/
  - apps/tinwave-desktop/README.md   # packaging/licence lines only
owner: tw-builder
branch: t/T-0058-switch-to-gpl
review_round: 1
created_by: from D-0009 (PO, 2026-11-03), ADR 0009
---

## Context
ADR 0009: Tinwave's own code is GPL-3.0-only (the root `LICENSE` already holds the GPL-3.0 text). The workspace still
declares `MIT OR Apache-2.0`, and the packaging (T-0042/T-0051) ships Apache-2.0 as Tinwave's licence and waits for
`LICENSE-MIT` / `LICENSE-APACHE` at the repository root in its D-0007 check (`common.ps1` `Get-ReleaseIdentity`,
`package.ps1` root-licence copy, `test-packaging.ps1`, packaging/README.md).

## Scope
- Root `Cargo.toml`: `license = "GPL-3.0-only"`.
- `deny.toml`: `[licenses.private] ignore = true` with a comment pointing at ADR 0009, so our own unpublished crates are
  not checked against the allow-list. The allow-list itself does **not** change (no GPL/LGPL added).
- Packaging:
  - ship the root `LICENSE` as Tinwave's licence (e.g. `licenses\Tinwave-LICENSE-GPL-3.0.txt`);
  - stop shipping `licenses\Apache-2.0.txt` *as Tinwave's licence* — keep it only if the third-party notices need it;
  - the D-0007 pending check stops looking for `LICENSE-MIT` / `LICENSE-APACHE`; it still requires the copyright holder
    and the source-offer contact (D-0007 stays open);
  - regenerate and commit `THIRD-PARTY-NOTICES.txt` (its "Tinwave's own source code is licensed under" line comes from
    cargo metadata);
  - update `test-packaging.ps1` cases and packaging/README.md to match.
- apps/tinwave-desktop/README.md: the packaging lines that mention Tinwave's licence.

## Acceptance criteria
- [ ] `cargo metadata` reports `GPL-3.0-only` for every workspace crate; `cargo deny check` passes with the allow-list unchanged.
- [ ] A negative check: adding a GPL dependency to any crate still fails `cargo deny` (show it, then revert).
- [ ] The package contains the GPL-3.0 text as Tinwave's licence and no MIT/Apache text presented as Tinwave's licence;
  `check-bundle` still reports D-0007 as PENDING (holder, contact) and no longer lists LICENSE-MIT / LICENSE-APACHE.
- [ ] `package.ps1` and `test-packaging.ps1` exit 0; the committed `THIRD-PARTY-NOTICES.txt` matches a fresh generation.
- [ ] fmt, clippy `-D warnings`, nextest, `cargo deny check` pass.

## Signals & storage
None.
