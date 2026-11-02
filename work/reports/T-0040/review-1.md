REVIEW T-0040 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass (`cargo fmt --all --check` exit 0), clippy pass (`--workspace --all-targets -D warnings` exit 0, includes the CXX-Qt-generated AppInfo bridge), tests pass (399 passed / 0 failed / 7 ignored; desktop: 4 unit + 2 `tests/startup.rs`), deny pass ("advisories ok, bans ok, licenses ok, sources ok"; only duplicate warnings for syn/thiserror/thiserror-impl/codespan-reporting/unicode-width, which are build-time deps from cxx/cxx-qt)

Worktree D:\dev\Tinwave-worktrees\T-0040-desktop-skeleton, HEAD 2fb85c2 (single commit; no fix commit yet for the lead's answers).

acceptance criteria:
- [x] Window opens in < 1 s warm; idle private working set reported. Verified by: the builder's release numbers (312 ms median, 39.0 MB PWS) plus my own spot check on the debug binary: window HWND visible after 484 ms, private WS 38.9 MB / WS 97 MB 4 s after it appeared. The process was killed and confirmed gone.
- [x] Token grep: no raw colours/sizes in QML outside `Theme/`. Verified by: grepping qml/ outside Theme/ for hex colours, pixelSize/pointSize, rgba/Qt.color, radius, duration and named colours, which found nothing. The only numeric layout literals are `spacing: 0` (Main.qml:38) and `padding: 0` (Main.qml:72), plus `currentIndex: 0` (Sidebar.qml:61). Theme.qml is byte-identical to spikes/T-0011-native-styles/qml/Theme/Theme.qml below the header comment.
- [x] QML load error ⇒ exit code ≠ 0 with a message, with a test. Verified by: `startup::missing_root_qml_exits_nonzero_with_a_message_instead_of_hanging` (asserts exit 2 and the message, with a 20 s hang guard), which passed in my run. The exit path is wiring.rs:47-55 (`on_object_creation_failed` → `process::exit(2)`).
- [x] Screenshots light + dark; keyboard walkthrough noted. Verified by: apps/tinwave-desktop/docs/screenshots/ (FluentWinUI3 light and dark, Fusion light, and a 7-step keyboard walkthrough showing Menu → Search → list focus ring → Up/Down selection → Tab wrap → Shift+Tab). Accessible names are at Sidebar.qml:112, 134 and 187.
- [x] fmt, clippy, tests and deny pass; generated bridge code is clippy-clean. Verified by: the reviewer runs above.

blocking findings:
- apps/tinwave-desktop/src/app/wiring.rs:57 — `TINWAVE_MAIN_QML` is read unconditionally, which goes against the lead's decision that it be honoured in debug builds only. Failure scenario: a shipped release build loads its root UI from whatever URL the environment supplies, such as `file:` or a network-mounted path, instead of the compiled-in `qrc:` shell. Fix: gate the env read (and `MAIN_QML_ENV`) behind `#[cfg(debug_assertions)]`, so release always loads `MAIN_QML_URL`; gate the load-failure test the same way, or `cargo test --release` would fail (default `cargo test` stays debug, so the test still runs); update README.md:34 to say "debug builds only".
- apps/tinwave-desktop/src/app/style.rs:12-15 (and test :40, module doc :4, README.md:14) — macOS gets `Fusion`, which goes against the lead's decision (design-system §2 wins: `macOS` style on macOS). Failure scenario: on a Mac the app draws Fusion controls instead of the native style §2 requires. Fix: add a `"macos" => "macOS"` arm with a comment that it is unverified until T-0038; keep `_ => "Fusion"` for Linux and others; change the test to assert `default_style_for("macos") == "macOS"` and `"linux" == "Fusion"`; update the doc comment and README.

design check: findings above (the style selection on macOS). Otherwise it passes: native ApplicationWindow, ItemDelegate, TextField, ToolButton, Menu and ToolSeparator are used unrestyled (only palette roles from tokens, Main.qml:23-25); no Tw* components; light and dark render; §4 groups are present. The accepted deviations (Menu text, missing hairline/radius/window-size/row-height tokens, native row height, reduced Sources/Listen items) were not counted against the branch.

Screenshot edge strips — a capture artefact, not a layout gap. Pixel evidence from all three shell PNGs (1822×1376):
- Columns x=0..10, columns x=1811..1821 and rows y=1365..1375 are exactly 11 px of pure (0,0,0). There is no strip at the top: row 0 is title-bar colour (242,243,249) in light and (32,32,32) in dark.
- The client area runs from x=11 to x=1810 (1800 px = 1200 logical × 1.5) and from y=45 to y=1364 (1320 px = 880 × 1.5). That is exactly the window size Main.qml:15-16 asks for at 150 % scaling. The first and last client pixels are already token colours: x=11 is `sidebar` #ECECEF / #1D1D20, x=1810 is `content`, y=1364 is `window` #F3F3F5 / #161618.
- 11 px is Windows 11's invisible resize border (7 logical px × 1.5) on the left, right and bottom only. It is included in GetWindowRect but not in DWMWA_EXTENDED_FRAME_BOUNDS, and PrintWindow/BitBlt fills it with black.
- The "lighter strip on the left" is the `sidebar` fill beginning right after the black border. It is not a gap. Main.qml's RowLayout uses `anchors.fill: parent` with `spacing: 0`, and the footer is the ApplicationWindow footer.
- Suggestion for future captures (not blocking): crop to DWMWA_EXTENDED_FRAME_BOUNDS.

non-blocking findings:
- apps/tinwave-desktop/qml/Main.qml:16 — `height: Theme.windowMinHeight + Theme.sidePanelWidth` uses a width token as a height increment just to avoid a literal. Replace it with a proper default-window-size token when T-0035 lands.
- apps/tinwave-desktop/qml/Sidebar.qml:115 — the app `Menu` has no `popupType`. Design-system §2 and ADR 0007 decision 1 call for `Popup.Native` with the dark-mode opt-in, falling back to `Popup.Window`. That needs the windows.rs bridge (not this ticket), so the Qt default is acceptable for now, but there is no comment recording the pending choice.
- apps/tinwave-desktop/qml/Main.qml:67-73 — the FluentWinUI3 `ToolSeparator` draws an inset line in the style's own colour (229,229,231), not the `separator` token. Cosmetic; resolves with TwTransportBar / a hairline token.
- apps/tinwave-desktop/src/app/wiring.rs:33-36 — release builds also honour `QT_QUICK_CONTROLS_STYLE` (Qt's own documented env behaviour, matches design-system §8). Note it next to the debug-only gate so the two env switches aren't confused.

proposals:
- P: Capture helper that crops to DWMWA_EXTENDED_FRAME_BOUNDS for UI screenshots | why: removes the black resize-border strips | size: S
- P: Record the app-menu popupType decision (Native + SetPreferredAppMode vs Window) in the platform-bridge ticket | why: ADR 0007 decision 1 is not yet wired | size: S

notes for lead: Both blocking findings are small. Round 2 should only need to re-check wiring.rs, style.rs, tests/startup.rs and the README. No scope violations. Signals: none required. No Windows settings touched, no ignored tests run, timing process killed.
