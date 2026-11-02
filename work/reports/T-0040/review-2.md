REVIEW T-0040 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass (`cargo fmt --all --check` exit 0), clippy pass (`--workspace --all-targets -D warnings` exit 0; `--release -p tinwave-desktop --all-targets -D warnings` exit 0), tests pass (`cargo test --workspace` exit 0 with 0 failed in every binary, desktop debug: 4 unit + 2 `tests/startup.rs`; `cargo test --release -p tinwave-desktop` exit 0: 4 unit + 1 startup, `window_shell_loads_without_qml_errors`), deny pass ("advisories ok, bans ok, licenses ok, sources ok"; only the known duplicate warnings for codespan-reporting/syn/thiserror/thiserror-impl/unicode-width)

Worktree D:\dev\Tinwave-worktrees\T-0040-desktop-skeleton, HEAD 6e2dba4. The 2fb85c2..6e2dba4 diff touches only apps/tinwave-desktop/ (README.md, qml/Sidebar.qml, src/app/style.rs, src/app/wiring.rs, tests/startup.rs). For the whole branch, main...branch, the only files outside apps/tinwave-desktop/ are Cargo.toml and Cargo.lock, and both are in the ticket scope.

acceptance criteria:
- [x] Window opens in < 1 s warm; idle private working set reported. Verified by: round-1 evidence (312 ms median and 39.0 MB private working set from the builder; my own spot check of 484 ms and 38.9 MB). The round-2 code changes cannot affect this.
- [x] Token grep: no raw colours/sizes in QML outside `Theme/`. Verified by: round-1 grep. The only QML change in round 2 is a two-line comment (Sidebar.qml:33-34).
- [x] QML load error ⇒ exit code ≠ 0 with a message, with a test. Verified by: `startup::missing_root_qml_exits_nonzero_with_a_message_instead_of_hanging` passed in the debug run (startup: 2 passed). The exit path at wiring.rs:54-63 is not gated, so release builds still exit with code 2 on a real load failure.
- [x] Screenshots light + dark; keyboard walkthrough noted. Verified by: round 1 (unchanged; the comment-only QML change needs no recapture).
- [x] fmt, clippy and tests pass in debug and release; deny passes; generated bridge code is clippy-clean. Verified by: the reviewer runs above.

Round-1 blocking findings:
1. Fixed. In wiring.rs:16-19, `MAIN_QML_ENV` is `#[cfg(debug_assertions)]`. In wiring.rs:27-34, `main_qml_url()` reads the env only under `#[cfg(debug_assertions)]` and otherwise returns `MAIN_QML_URL`. In tests/startup.rs:47 and :56-58, `read_stderr` and the load-failure test are gated the same way. README.md:34 now says "**debug builds only**". I also checked the release binary directly. I ran target\release\tinwave-desktop.exe headless with `TINWAVE_MAIN_QML=qrc:/tinwave-test/DoesNotExist.qml`. After 3 s it was still running (the override was ignored and the shell loaded), and its stderr did not mention DoesNotExist. I killed it; 0 tinwave-desktop processes remained.
2. Fixed. style.rs:15-16 adds the `"macos" => "macOS"` arm with the comment "Unverified on a Mac until T-0038", and `_ => "Fusion"` is kept. The test `macos_selects_its_native_style_and_linux_uses_fusion` (style.rs:41-44) passes in debug and release. The module doc (style.rs:4-5) and README.md:14 are updated.

Round-1 non-blocking items addressed:
- Sidebar.qml:33-34 has a comment on the app Menu recording that popupType is pending ADR 0007 decision 1 (Native plus the dark opt-in, falling back to Window), to be wired with the platform bridge in T-0036. Accepted.
- The `QT_QUICK_CONTROLS_STYLE` note is now in the doc comment at style.rs:7-8 and at README.md:32 ("Qt's own documented switch, honoured in release builds too (design-system §8)"). Accepted.

blocking findings:
- none

design check: pass. macOS now selects the §2 native style. Round-1 design observations are unchanged and remain non-blocking.

non-blocking findings:
- apps/tinwave-desktop/qml/Main.qml:16: the width token is still used as a height increment. Carry-over from round 1; resolve with T-0035.
- apps/tinwave-desktop/qml/Main.qml:67-73: the FluentWinUI3 ToolSeparator colour is not the `separator` token. Carry-over; resolve with TwTransportBar or a hairline token.
- The ticket body still says "Fusion elsewhere" (T-0040 Scope, line 31), while the code now follows the lead's macOS decision. If you want the ticket text to match, update it when you close the ticket.

proposals:
- P: Capture helper that crops to DWMWA_EXTENDED_FRAME_BOUNDS for UI screenshots | why: removes the black resize-border strips | size: S
- P: Record the app-menu popupType decision (Native + SetPreferredAppMode vs Window) in the T-0036 platform-bridge ticket | why: ADR 0007 decision 1 is not yet wired; the comment now points at T-0036 | size: S

notes for lead: Ready to merge. The builder's round-1 proposal "decide the macOS style" is now moot and can be dropped. In a `--release` test run the load-failure test does not exist by design; it runs in the default debug `cargo test` and in CI. No Windows settings were touched and no ignored tests were run. The one tinwave-desktop process I started (release, offscreen) was killed and confirmed gone.
