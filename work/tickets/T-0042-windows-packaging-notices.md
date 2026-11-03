---
id: T-0042
title: Windows packaging — windeployqt recipe, THIRD-PARTY-NOTICES (Qt LGPL-3), installer check
type: chore
epic: E-01
status: in_review
priority: P1
size: M
depends_on: [T-0040]
scope:
  - apps/tinwave-desktop/packaging/
owner: tw-builder
branch: t/T-0042-windows-packaging-notices
review_round: 1
created_by: from T-0002
---

## Context
ADR 0008: dynamic Qt only; trimmed windeployqt bundle 55.4 MiB (FluentWinUI3 needs Fusion deployed); LGPL-3 texts,
notices for third-party code inside Qt (FreeType, PSL data, …), Qt source offer; never ship GPL-only test-utils.

## Acceptance criteria
- [ ] Scripted deploy produces a self-contained folder that runs with Qt off PATH.
- [ ] THIRD-PARTY-NOTICES generated (qtattributionsscanner + cargo-deny/about for crates) and included.
- [ ] A check fails the build if a GPL-only Qt module or a static Qt is present.

## Signals & storage
None.

## Note from T-0044 review-1
- `cargo build --release --workspace` also builds the dev-only `image-stress.exe`; the package must ship
  `tinwave-desktop.exe` only (never copy `target/release/*.exe`).

## Lead notes (2026-11-03, dispatch)
- Generate the notices into packaging/; read other notices sources (e.g. the Lucide ISC text T-0043 adds under
  apps/tinwave-desktop/icons/) by path, do not edit them.
- Do not install new tools without saying so first: if cargo-about (or similar) is not installed, use what is
  (`cargo deny`, `cargo metadata`, Qt's qtattributionsscanner) and propose the tool in the handoff.
