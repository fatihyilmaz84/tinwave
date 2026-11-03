REVIEW T-0036 (round 1)
verdict: APPROVE
checks (run by reviewer): fmt pass, clippy pass (workspace --all-targets -D warnings), tests pass (cargo nextest run --workspace: 543 passed, 0 failed, 6 skipped), deny pass
merge-tree: `git merge-tree --write-tree main t/T-0036-windows-platform-bridge` → 1df15490…, clean. Worktree HEAD bde9a72, clean.

acceptance criteria:
- [x] Each provider unit-tested, live OS toggles left to the PO runbook: controller with a fake platform — 24 tests in src/platform/controller.rs (backdrop blockers: transparency, high contrast, `--transparency`; menu fallback and retry; high contrast forcing FollowSystem; keeping last values after a failed read; a failed watch). `platform::windows::tests` (15 pass): the real read-only SettingsReader, build gates, both uxtheme exports resolving, CRT environment round trip, "OS-only env var is invisible to the CRT". Watcher tests send synthetic WM_SETTINGCHANGE, WM_THEMECHANGED, WM_SYSCOLORCHANGE and 0x320 with `SendMessageW` to their own `HWND_MESSAGE` STATIC window; "watch again replaces the callback"; "destroy removes the hook". unsupported.rs `every_provider_says_unsupported`. tests/platform_qml.rs (4 pass).
- [x] unsafe only in one documented module with SAFETY per block; non-Windows Unsupported: `unsafe` only in platform/windows.rs (`#[allow(unsafe_code)]` on the `mod` declaration, platform/mod.rs:28-30) and the bridge modules; platform_bridge.rs has one SAFETY-commented `window.as_ref()` plus the cxx declarations, allow on app/mod.rs:13-18 (ADR 0008 decision 4). `unsafe_code = "deny"` and `undocumented_unsafe_blocks = "deny"` pass. SAFETY comments checked by hand and accurate (except the `live_window` doc, below). unsupported.rs answers `Unsupported` everywhere via `platform::current()`.
- [x] P-0121: Sidebar.qml:43 `popupType: PlatformSettings.nativeMenus ? Popup.Native : Popup.Window`; probe asserts nativeMenus=true, menuStatus=native. Ran the app with `--color-scheme=dark`, opened the menu (Tab+Space): native Win32 menu drawn dark. Quit through the native menu (Tab, Space, Up, Enter) exited with code 0.
- [x] P-0132: `--accent=#rrggbb` in debug and release, in-process only; validation regex in Theme.qml:37 (`setAccentOverride`); bad values warn. Tests `developer_switches_change_the_app_in_process_only`, `bad_switch_values_are_reported_and_ignored`. Ran `--color-scheme=dark --accent=#FFB900 --transparency=off`: solid grey sidebar, amber selection indicator, empty stderr.
- [x] Screenshots: all 9 reviewed (light/dark Mica, light/dark solid, FFB900 light, 1F3A93 dark, dark native menu open, light native menu on a dark OS — ForceLight works, focus ring after Esc — T-0050 ring intact); match design-system §2 rows.

Focus items:
- WM_SETTINGCHANGE hook: the HOOKS lock is never held across `CallWindowProcW` or the callback; on WM_NCDESTROY the hook is removed and the original proc still sees the message (no callback after destroy); the callback only calls `qt_thread().queue` — after PlatformSettings is gone queue returns Err, ignored — safe after UiOwner teardown; the raw QQuickWindow pointer is used only inside `attachWindow` (winId read, not kept).
- uxtheme ordinals 135/136: `LoadLibraryExW(LOAD_LIBRARY_SEARCH_SYSTEM32)`, by ordinal at run time; gated to >= 18362; any miss → `Unsupported` → `nativeMenus` false → `Popup.Window`. High contrast forces AllowDark (controller.rs:189-195, tested).
- CRT `_putenv_s`: called from `wiring::run()` before `Shell::start` creates the `QGuiApplication`; no crt-static anywhere, so Rust and Qt share the UCRT environment; if missing, `set_backdrop(on)` refuses with `Unsupported` → solid sidebar with a reason, never white.
- No OS setting written: grep for BROADCAST, SPI_SET, RegSet, SPIF_, SetSysColors, PostMessage finds nothing; only reads, the process app mode and our own window's DWM attributes.
- Theme inputs: `reducedMotion` makes `durationFast/Normal/Slow` 0; `highContrast` switches `resolve()` to the system palette and blocks the backdrop.
- T-0040/T-0044/T-0050: startup.rs, image_provider.rs, tests/common unchanged; exit-2 test, window-shell test and cancel_storm pass.

blocking findings:
- none

design check: pass. No hex colours or numeric `font.pixelSize` in qml/ outside Theme/; no new token, Tw* or restyled native control; light/dark, non-default accent, solid fallback and native menu in evidence and on screen; keyboard path verified.

non-blocking findings:
- src/app/platform_bridge.rs:20 — `fn winId(self: &QQuickWindow) -> usize`: WId is `quintptr` = `quint64` (qtypes.h:230); cxx maps `usize` to `std::size_t` — matches on MSVC, but on 64-bit Linux/macOS `size_t` is `unsigned long`, so the generated binding very likely won't compile there (`u64` has the same issue). The "clippy on x86_64-unknown-linux-gnu" check couldn't have compiled the C++ side. Not blocking (no CI; other OSes are T-0038/T-0039). Fix: a one-line C++ shim returning `static_cast<rust::usize>(w.winId())`.
- src/platform/windows.rs:180-193 — the doc says a "foreign number must not reach DWM", but `IsWindow` accepts any process's window. Today the handle is our own `winId()`; on the offscreen QPA winId is a small counter passed as an HWND (failed `IsWindow` here; 1..40 none is a window; the probe even accepts "live"). Check `GetWindowThreadProcessId(hwnd) == GetCurrentProcessId()` in `live_window`, or attach only on the "windows" QPA.
- tests/platform_qml.rs:85-90 — `durationFast` only checked to be in {0, 120}; `themeReducedMotionFollows` passes trivially when both are false. Tighter: durationFast == (reducedMotion ? 0 : 120). Controller tests cover the logic.
- Scope: src/lib.rs (`pub mod platform`), build.rs (registering the bridge file), Cargo.lock outside the listed paths — needed wiring.
- src/platform/windows.rs:442 — every `set_menu_scheme` adds a uxtheme.dll load reference, never released (harmless; a `OnceLock` of the two function pointers would be tidier).

proposals:
- P: C++ shim for `QWindow::winId` returning `rust::usize` so the bridge compiles on Linux/macOS | size: S
- P: Ownership guard in `live_window` (same process) plus "attach only on the windows QPA" | size: S
- P: Re-apply `set_menu_scheme` (FlushMenuThemes) on an OS colour-scheme change if the PO runbook shows stale native menus after a live light/dark switch (with "system", an OS toggle doesn't re-flush today) | size: S

notes for lead: live OS toggles not exercised by anyone (no setting may be changed). In the PO runbook, check that the native menu follows an OS light→dark switch while the app runs with "system" scheme; if uxtheme caches the mode, that is a light menu in a dark app (D-0006 forbids) — the third proposal fixes it. Checks ran against bde9a72 (clippy cached no-op, nextest full). No tinwave-desktop left; PID 26052 cargo is someone else's tinwave-playback stress run. Reviewer scripts and two PNGs remain in the session scratchpad (delete blocked as a protected path); not in the repo.

---
Lead (2026-11-03): merge. Non-blocking items and P-0194..P-0196 → T-0053 (platform bridge follow-ups, P2); the live
light→dark menu check is added to the PO runbook note on the board. P-0190 (design-system text) applied on merge.
