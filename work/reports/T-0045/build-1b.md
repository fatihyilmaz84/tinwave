HANDOFF T-0045 (round 1, post-approval)
status: done
branch: t/T-0045-contrast-selfcheck-test   commit: ca166d4 (on top of 8f454c6)

1. Independent ratio check: tests/qml_probes.rs parses each row's printed text>=, accentText>=, onAccent>= and
   fixedLabel/accentFill= values and asserts each >= 4.50 in Rust (0.005 allowance: rows print two decimals),
   regardless of the row's own ok/FAIL. Rows print no separate glyph ratio; accentText carries the glyph/focus-ring bar
   and 4.5 covers the 3:1 requirement (stated in a test comment).
2. Mutation (reviewer's): Theme.qml:216 minima -> 3.0 and accentText target (line 170) -> 3.0. The check said "ok" for
   every row, but the test failed: "ratios below the WCAG bar, whatever the check said: accentText>=4.43 < 4.5 in:
   light #0067C0 ... ok; accentText>=3.01 < 4.5 in: light #FFB900 ... ok ...". Theme.qml reverted.
3. Stale header comments in tests/qml/ContrastProbe.qml:1 and tests/qml/IconProbe.qml:1 now name tests/qml_probes.rs.

checks: fmt pass, clippy pass, tests 545 passed / 0 failed (6 skipped), deny pass.
files changed: apps/tinwave-desktop/tests/qml_probes.rs, tests/qml/ContrastProbe.qml, tests/qml/IconProbe.qml
No tinwave-desktop process left running.

---
Lead (2026-11-03): diff 8f454c6..ca166d4 checked (3 files, test-only); merge after main's T-0023 gate run.
