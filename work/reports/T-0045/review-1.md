REVIEW T-0045 (round 1)
verdict: APPROVE
checks (run by reviewer, worktree, Qt 6.12 on PATH): fmt pass, clippy -D warnings pass, tests pass (`cargo nextest run --workspace`: 545 run, 545 passed, 6 skipped; qml_probes contrast 0.373 s, icon 0.503 s; the 4 platform_qml tests pass), deny pass. `git merge-tree --write-tree main t/T-0045-contrast-selfcheck-test` clean (tree c135790e). One commit, 8f454c6.

acceptance criteria:
- [x] Passes on main; restoring light textFaint #6E6E77 fails readably — re-ran in a scratch `git archive` copy with its own CARGO_TARGET_DIR: "contrast below 4.5:1 (design-system.md 3.1), 15 of 30 rows:" listing all 15 light rows (e.g. `light #0067C0 text>=3.94 (textFaint on hover/sidebar) ... FAIL`). Second mutation (dark textMuted #6E6E77, accentText target 4.5 → 3.0): 28 of 30 rows fail; dark rows `text>=2.72 (textMuted on hover/raised)`, light rows `accentText>=3.00…4.43`; the two light accents still ≥ 4.5 without the push (#0B6E78, #000000) stayed ok. The message points at the failing pair and ratio.
- [x] Default test run, no display/audio, < 10 s — plain `cargo nextest run --workspace`, ~0.37 s, offscreen + software backend; tinwave-desktop has no audio dependency. `#![cfg(debug_assertions)]` justified (`MAIN_QML_ENV` handling is debug-only, wiring.rs:31-43).
- [x] fmt, clippy -D warnings, tests, deny pass.
- [x] P-0156 TwIcon probe — asserts Ready, source ending `/icons/menu.svg`, implicit and sourceSize 16×16, nameless icon loads nothing at implicit 20. Stderr filter tested: `opacity: noSuchThing ? 1 : 1` in TwIcon.qml (scratch) keeps status Ready, test fails with "QML reported problems: qrc:/qt/qml/Tinwave/Desktop/qml/Tw/TwIcon.qml:33: ReferenceError…". The `PROBE ` exclusion only skips the probe's own console.log lines.
- [x] §3.1 names the test (design-system.md:103-108: test, file, probe root, debug-only, headless, well under a second); row format matches; no other doc still describes the hand harness.

blocking findings:
- none

design check: n/a (test-only)

non-blocking findings:
- tests/qml_probes.rs:31-42 — the guards (21.00 / 1.00 / 15 accents / 30 rows) catch an emptied list and broken WCAG maths, but not a weakened verdict: the test trusts selfCheck's own `ok`/`FAIL`. Confirmed in scratch: Theme.qml:216 changed to `textMin >= 3.0 && accentTextMin >= 3.0 && …` and the accentText target lowered to 3.0 → both tests pass with rows showing accentText 3.00. Not blocking (the gate is visible in Theme.qml diffs); cheap fix: parse each row's four printed ratios in Rust and assert each ≥ 4.50 (glyph ≥ 3.00) independently.
- tests/qml/ContrastProbe.qml:1 and tests/qml/IconProbe.qml:1 — stale header comments name `tests/contrast_qml.rs` and `tests/icon_qml.rs`; the runner is `tests/qml_probes.rs`.
- Shared runner refactor keeps platform_qml behaviour: `facts` changed from BTreeMap (last wins) to Vec (`get` returns first); PlatformProbe.qml prints each name once, so results are identical.

proposals:
- P: Re-check the printed ratios in Rust (each ≥ 4.50) in the contrast probe | size: S

notes for lead: scope clean; scratch copy deleted; worktree untouched; no tinwave-desktop left. Other processes on the machine: several python3.exe from earlier sessions, cargo PID 62472 (`cargo test -q -p tinwave-playback --test queue_edits`, another agent).

---
Lead (2026-11-03): approve stands. Before merge the builder adds the Rust-side ratio re-check (P-0208, merged here)
and fixes the two header comments; the lead verifies the diff, then merges.
