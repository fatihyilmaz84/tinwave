HANDOFF T-0053 (round 1)
status: done
branch: t/T-0053-platform-bridge-followups   commit: 06121e6
summary: The bridge reads the window handle through a C++ shim returning rust::usize and attaches only on the `windows` QPA. `live_window` refuses windows owned by another process. The uxtheme entry points are resolved once in a OnceLock, and menus that follow the system are re-flushed on every OS settings/theme/colour message. platform_qml asserts durationFast exactly against the OS value. The offscreen run records "not attached (QPA offscreen)" instead of "IsWindow failed". No Windows setting was changed: the only calls on foreign windows are IsWindow and GetWindowThreadProcessId (reads).

acceptance criteria:
- [x] Ownership guard and re-flush tests — `windows::tests::only_a_window_of_this_process_is_accepted` (accepts our own message-only window; refuses GetDesktopWindow and GetShellWindow, owner read only); `a_foreign_window_is_neither_watched_nor_given_a_backdrop` (desktop window not subclassed, `set_backdrop` refused). Re-flush: controller `an_os_change_while_menus_follow_the_system_flushes_them_again`, `an_os_change_does_not_touch_menus_forced_to_a_scheme`; `windows::tests::an_os_scheme_message_reflushes_native_menus_that_follow_the_system` end-to-end on the real watcher and real uxtheme calls with a counting wrapper, sending WM_NULL, WM_SETTINGCHANGE, WM_THEMECHANGED to its own message-only window only.
- [x] Offscreen records "not attached (QPA offscreen)" — `platform_qml::windows_reads_the_os_and_offers_native_menus` asserts settingsStatus "not attached (QPA offscreen)", backdropStatus "solid: not attached (QPA offscreen)", no "IsWindow" on stderr; controller `a_window_that_is_not_an_os_window_is_reported_not_attached_and_nothing_is_called`.
- [x] fmt, clippy -D warnings, tests, deny pass; no Windows setting changed (only the existing process-local uxtheme app mode).
- [x] uxtheme pointers resolved once — `menu_api()` caches success in a `OnceLock` (`the_uxtheme_entry_points_are_resolved_once`); a failure isn't cached (the controller retries a failed opt-in).
- [x] Exact duration assertion — `durationFast == (os.reduced_motion ? "0" : "120")` with `os = platform::current().read().unwrap_or_default()`.

checks: fmt pass, clippy pass, tests 580 passed / 0 failed (6 skipped), deny pass. A second unchanged `cargo build -p tinwave-desktop` compiled nothing (0.13 s).
files changed: apps/tinwave-desktop/cpp/platform_window.h (new), cpp/platform_window.cpp (new), build.rs (one `.cpp_file` line), src/app/platform_bridge.rs, src/platform/mod.rs, src/platform/controller.rs, src/platform/windows.rs, tests/platform_qml.rs
signals added/changed: none
ui: none
docs updated: doc comments in the touched files

deviations from ticket:
- `std::process::id()` (= GetCurrentProcessId) instead of a new `windows` feature (Cargo.toml out of scope); the new unsafe is one SAFETY-commented `GetWindowThreadProcessId` block in windows.rs.
- The re-flush fires on every OS change message while menus follow the system (the message doesn't say what changed; FlushMenuThemes is cheap and idempotent); forced Dark/Light isn't re-flushed.

winId type reasoning: MSVC `WId` = `quintptr` = `unsigned long long`; `rust::usize` = `std::size_t` = `unsigned long long` on x64 → same-type cast, identical HWND (verified: guard accepts a real window, QML tests pass). 64-bit Linux/macOS: `quint64` = `unsigned long long`, `size_t` = `unsigned long` — same width, different type (why cxx's direct binding fails); `static_cast` between them is valid. 32-bit: `quintptr` = `size_t`. Not compiled on Linux/macOS.

Other points:
- The shim also exposes `tw_qpa_name()` (`QGuiApplication::platformName()` as `rust::String`); `platform::has_native_window_handles(qpa)`: on Windows only "windows"; other OSes true (Unsupported providers answer with their own reasons); T-0038/T-0039 must narrow it.
- Ownership guard returns `Failed { call: "window owned by another process", code: 1400 }` (no new `PlatformError` variant).
- Mutations (each failed the named tests, restored): dropping `owner == process::id()`; disabling the re-flush; `menu_api` leaking a fresh resolution per call; `has_native_window_handles` always true (IsWindow reappears); Theme `durationFast` swapped to `reducedMotion ? 120 : 0` (the old `{0,120}` assertion would have passed).
- `developer_switches_change_the_app_in_process_only` now expects backdropStatus "solid: not attached (QPA offscreen)"; the switch is still proven via `transparencyEffects == "false"` and controller tests.
- A combined restore-and-cleanup command was blocked, so mutations stayed in the tree briefly; restored from git and by hand, then fmt, clippy, full nextest and deny re-run on the restored tree. The commit contains none of them.
- No processes left.

proposals:
- P: Exact re-flush (flush only on a light/dark change; read `AppsUseLightTheme` into `SystemSettings`) | size: S
- P: Narrow `has_native_window_handles` per OS in T-0038/T-0039 | size: S

questions for lead: none

---
Lead (2026-11-03): P-0215 (exact re-flush) parked — only if a settings storm ever matters. P-0216 merged into the
T-0038/T-0039 notes (with P-0188).
