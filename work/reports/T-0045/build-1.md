HANDOFF T-0045 (round 1)
status: done
branch: t/T-0045-contrast-selfcheck-test   commit: 8f454c6
summary: `Theme.selfCheck()` (15 accents x light/dark) runs as a headless test in the default `cargo test`, and a second headless test loads a `TwIcon` scene and checks for QML warnings. Both reuse the `TINWAVE_MAIN_QML` probe pattern; the probe runner moved out of `platform_qml.rs` into a shared helper. Design-system §3.1 names the test instead of the hand procedure.

acceptance criteria:
- [x] Test passes on main; restoring light textFaint #6E6E77 makes it fail readably — `qml_probes::every_check_accent_keeps_every_token_readable_in_both_schemes` passes unmutated; with #6E6E77 it fails with "contrast below 4.5:1 (design-system.md 3.1), 15 of 30 rows:" and rows like `light #0067C0 text>=3.94 (textFaint on hover/sidebar) accentText>=4.51 onAccent>=5.67 fixedLabel/accentFill=5.67 FAIL`. Theme.qml reverted.
- [x] Runs in the default test run without a display or audio, < 10 s — offscreen + software backend; contrast ~0.2-0.3 s, icon ~0.5 s; both inside the full `cargo nextest run --workspace` (545 passed).
- [x] fmt, clippy -D warnings, tests, deny pass.
- [x] P-0156: `qml_probes::tw_icon_loads_its_svg_and_sizes_itself_without_qml_warnings` loads a TwIcon (`menu`, size 16) in an offscreen Window: status Ready, source ends `/icons/menu.svg`, implicit and source sizes 16; a nameless TwIcon loads nothing, implicit size 20; no stderr `qrc:` lines apart from the probe's own `PROBE` lines. Mutation: name "nope" fails (`status-3`, "QML IconImage: Cannot open: .../icons/nope.svg"). Reverted.

checks: fmt pass, clippy pass, tests 545 passed / 0 failed (6 skipped), deny pass
files changed: apps/tinwave-desktop/tests/qml_probes.rs (new); tests/qml/ContrastProbe.qml (new); tests/qml/IconProbe.qml (new); tests/common/probe.rs (new shared runner); tests/common/mod.rs (`pub mod probe`, debug only, `allow(dead_code)`); tests/platform_qml.rs (uses the shared runner; tests unchanged, pass); docs/design/design-system.md (§3.1 "The check")
signals added/changed: none
ui: none (test-only)
docs updated: design-system.md §3.1
deviations from ticket: none. ContrastProbe also asserts black-on-white 21.00, white-on-white 1.00, 15 accents and 30 rows (so "0 failures" can't pass on a gutted check). Tests are `#![cfg(debug_assertions)]` like platform_qml (`TINWAVE_MAIN_QML` is debug-only); `cargo test --release` skips them. `tests/qml/` isn't scanned by `check_icon_usage` (reads `qml/` only).

proposals:
- P: Cover high-contrast mode in the headless contrast check (selfCheck resolves with hc=false only; the HC branch uses system-palette colours) | size: S
- P: Assert TwIcon rendered pixels (tint) via `grabToImage` | size: S

questions for lead: none. No background processes; no tinwave-desktop left.

---
Lead (2026-11-03): P-0202 (HC contrast) parked — under high contrast the OS palette owns contrast; revisit if Theme
adds HC-specific tokens. P-0203 (pixel tint) parked → first screen that uses TwIcon.
