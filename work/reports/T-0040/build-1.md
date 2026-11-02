HANDOFF T-0040 (round 1)
status: done
branch: t/T-0040-desktop-skeleton   commit: 2fb85c2
summary: New crate apps/tinwave-desktop (library + thin binary, CXX-Qt 0.10, Cargo-driven, dynamic Qt) opens a FluentWinUI3 window shell. The shell has a native title bar, a 232 sidebar with the four section-4 groups as ItemDelegates, an empty content area, and a transport-bar placeholder. Theme.qml is ported unchanged from T-0011 (comments only; no T-0035 fixes). A QML load failure exits with code 2 and a message, and is covered by a headless test. Measured on release: window visible at 312 ms median (n=8, range 247-354), idle private working set 39 MB.

acceptance criteria:
- [x] Window opens in < 1 s warm; idle private working set reported — proof: release build, 8 runs, Qt bin on PATH, warm.
  - Process start to main window HWND visible: 312 ms median [247..354]. The dev build was 236-297 ms. Measured externally by polling MainWindowHandle, so first frame is slightly later.
  - Idle private working set 4 s after the window appeared: 39.0 MB median [38.6..39.4] (target <= 60 MB). Total working set is about 98 MB.
  - Fusion style (QT_QUICK_CONTROLS_STYLE=Fusion): 25.8 MB private, 83 MB working set.
- [x] Token grep: no raw colours/sizes in QML outside Theme/ — proof: grep for `#hex`, `pixelSize`, `rgba(`, `radius`, `duration`, and `width|height|spacing|margins|padding: <number>` over qml/ excluding Theme/. The only hits are literal `0` values (`spacing: 0`, `padding: 0`, popup offset 0, `currentIndex: 0`). Window size, gaps and fonts all come from Theme tokens.
- [x] QML load error => exit code != 0 with a message — proof: `cargo test -p tinwave-desktop --test startup`, test `missing_root_qml_exits_nonzero_with_a_message_instead_of_hanging`. It asserts exit code 2 and the message "failed to load the user interface ... DoesNotExist.qml", with a 20 s hang guard.
  - Companion test `window_shell_loads_without_qml_errors` fails on any `qrc:` warning in stderr. Verified it catches an injected `ReferenceError` (mutation reverted).
- [x] Screenshots light + dark; keyboard walkthrough noted — proof: files under apps/tinwave-desktop/docs/screenshots/.
- [x] fmt, clippy -D warnings, tests, cargo deny; generated bridge code clippy-clean — proof: `cargo fmt --all --check` ok; `cargo clippy --workspace --all-targets -- -D warnings` clean (includes the CXX-Qt-generated AppInfo bridge); `cargo test --workspace` about 399 passed, 0 failed (desktop crate: 4 unit + 2 integration); `cargo deny check` ok.

checks: fmt pass, clippy pass, tests ~399 passed / 0 failed, deny pass
files changed:
- Cargo.toml (members += "apps/*"; workspace deps cxx, cxx-qt, cxx-qt-build, cxx-qt-lib), Cargo.lock
- apps/tinwave-desktop/Cargo.toml, build.rs, README.md
- apps/tinwave-desktop/src/lib.rs, main.rs
- apps/tinwave-desktop/src/app/mod.rs, wiring.rs, style.rs, app_info.rs
- apps/tinwave-desktop/qml/Main.qml, Sidebar.qml, Theme/Theme.qml
- apps/tinwave-desktop/tests/startup.rs
- apps/tinwave-desktop/docs/screenshots/*.png (4 files)
signals added/changed: none (no playback or preference interaction yet)

ui:
- Design ref: design-system §2, §3, §4 and canvas "Library · Windows" (shell only).
- Native controls: ApplicationWindow, ItemDelegate (in a ListView with section headers), TextField (Search), ToolButton + Menu (app menu), ScrollBar, ToolSeparator (hairline above the transport bar). Tw components: none. Every colour, size, gap and font comes from Theme.
- OS checked: Windows 11 26200, Qt 6.12.0 MSVC, FluentWinUI3, 150% scaling. Also Fusion via QT_QUICK_CONTROLS_STYLE.
- Screenshots: shell-windows11-fluentwinui3-light.png, shell-windows11-fluentwinui3-dark.png, shell-windows11-fusion-light.png, keyboard-walkthrough-windows11-fluentwinui3-light.png (sidebar crops, 7 steps).
- Scheme switching: in-process via `--color-scheme=light|dark|system`, applied through `Theme.setSchemeOverride`. The OS theme was never touched.
- Keyboard walkthrough: Tab goes Menu → Search → nav list current item (native focus ring); the list is one tab stop (StrongFocus on current, ClickFocus on others); Up/Down move the selection with highlight and accent pill; Tab out wraps to Menu, Shift+Tab returns; content and transport bar have nothing interactive yet; the ring is not drawn during arrow navigation; accessible names on menu button, search field, every nav item.
- Not checked: non-default OS accent, macOS/Linux styles (only Fusion on Windows).
- Deviations from the design: "Menu" text instead of the hamburger icon (no Lucide infrastructure, §3.4); no rounded top-left content corner or 1 px canvas borders (missing tokens); sidebar rows use native ItemDelegate height (40 px) not the canvas's 34 px; Sources shows only "This computer" and "Downloads", Listen only "Smart Radio" and "Insights".

docs updated: apps/tinwave-desktop/README.md added (build/run/test env, dev switches, exit codes).

deviations from ticket:
- Added `AppInfo`, a minimal QML singleton view model (`version` constant, shown in the app menu), to prove the bridge lint pattern and generated-code clippy criterion (~30 lines).
- Crate is lib + bin instead of bin-only (bin-only failed to link the integration test, LNK2019).
- Non-Windows style is Fusion as the ticket says; design-system §2 says macOS uses the `macOS` style.
- Bridge `#[allow(unsafe_code)]` is on the `mod app_info;` declaration in app/mod.rs; cxx-qt rejects attributes other than `doc` and `cxx_qt::bridge` on the bridge module.
- Dev-only switches documented in the README: `TINWAVE_MAIN_QML`, `--color-scheme=`, and `QT_QUICK_CONTROLS_STYLE` honoured by skipping `set_style`.

findings worth recording:
- Any binary linking the crate (app and `cargo test`) needs the Qt bin dir on PATH, otherwise STATUS_DLL_NOT_FOUND (0xc0000135).
- Qt 6.12 writes to a piped stderr only with `QT_FORCE_STDERR_LOGGING=1`.
- 150% screenshot capture needed a DPI-aware capture process.
- QML mnemonics: literal `&` in ItemDelegate text must be `&&`.
- Mica/platform bridge and T-0035 token fixes untouched; no Windows setting changed; no Credential Manager; no audio device; all processes closed.

proposals:
- P: Update ADR 0008 / qml-ui.md / desktop-dev-setup.md with the bridge lessons | size: S
- P: Icon pipeline ticket (Lucide SVGs in the Qt resource file, a Tw icon helper) | size: M
- P: Design ticket for missing tokens (hairline width, default window size, content-surface corner radius) | size: S
- P: Decide the macOS style (`macOS` per design-system §2 vs Fusion per ticket) | size: S
- P: `cargo xtask`/script or `.cargo/config.toml` `[env]` for QMAKE and Qt PATH | size: S
- P: QML test hook for first-frame time | size: S

questions for lead:
- Is the unconditional `TINWAVE_MAIN_QML` developer override acceptable in release builds?
- The headless tests run in default `cargo test` (offscreen, no display, no audio); acceptable given they need Qt DLLs on PATH?

---
Lead (2026-11-02): TINWAVE_MAIN_QML — gate it to debug builds (`cfg(debug_assertions)`); the default `cargo test` is a debug
build, so the load-failure test still runs. Release builds must not load UI from an arbitrary URL. Headless tests in the
default run: accepted (the workspace needs Qt to build anyway since ADR 0008); documented in desktop-dev-setup by the lead.
macOS style: design-system §2 wins (`macOS` on macOS), verified in T-0038; the code may select it now with a comment.
