---
id: T-0045
title: Run Theme.selfCheck() as a committed test (15 accents × 2 schemes)
type: feature
epic: E-01
status: in_progress
priority: P2
size: S
depends_on: [T-0035]
scope:
  - apps/tinwave-desktop/tests/
  - apps/tinwave-desktop/tests/qml/          # harness QML, if needed
  - docs/design/design-system.md             # §3.1 "The check": point at the test
owner: tw-builder
branch: t/T-0045-contrast-selfcheck-test
review_round: 1
created_by: from P-0129
---

## Context
T-0035 added `Theme.selfCheck(accents)` (design-system §3.1) and ran it from a scratch `qml` harness by hand.
Token edits must not silently break contrast (text ≥ 4.5:1, UI glyphs ≥ 3:1 on every surface incl. hover/selected).

## Scope
- A headless test in the default `cargo test` (offscreen platform, like tests/startup.rs) that loads Theme and runs
  `selfCheck` over the 15-accent list for light and dark, failing on any failure row with the pair and ratio.
- §3.1 names the test instead of the hand procedure.

## Acceptance criteria
- [ ] Test passes on main; mutation: restoring light textFaint #6E6E77 makes it fail with a readable message.
- [ ] Runs in the default test run without a display or audio; < 10 s.
- [ ] fmt, clippy -D warnings, tests, deny pass.

## Signals & storage
None.

## Added from P-0156 (T-0043 build-1)
- Also a headless probe that loads a `TwIcon` scene through `TINWAVE_MAIN_QML` (debug) and asserts no `qrc:` warnings,
  so TwIcon has coverage while the app does not use it yet.
