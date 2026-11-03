REVIEW T-0053 (round 1)
verdict: CHANGES_REQUESTED
checks (run by reviewer): fmt pass; clippy pass (workspace, all targets, -D warnings; cached on a clean tree); tests pass (cargo nextest run --workspace: 580 run, 580 passed, 6 skipped); deny pass
merge-tree: `git merge-tree --write-tree main t/T-0053-platform-bridge-followups` → 8a6bcd04…, no conflicts. Worktree HEAD 06121e6, clean.

acceptance criteria:
- [x] Ownership guard and re-flush tests — `platform::windows::tests::only_a_window_of_this_process_is_accepted`, `a_foreign_window_is_neither_watched_nor_given_a_backdrop`; `controller::tests::an_os_change_while_menus_follow_the_system_flushes_them_again`, `an_os_change_does_not_touch_menus_forced_to_a_scheme`; `windows::tests::an_os_scheme_message_reflushes_native_menus_that_follow_the_system` (real watcher and uxtheme calls, messages only to its own HWND_MESSAGE window). All pass.
- [x] Offscreen records "not attached (QPA offscreen)" — `platform_qml::windows_reads_the_os_and_offers_native_menus` asserts settingsStatus and backdropStatus exactly and no "IsWindow" on stderr; `controller::tests::a_window_that_is_not_an_os_window_is_reported_not_attached_and_nothing_is_called`.
- [x] fmt, clippy, tests, deny pass; no Windows setting changed — only IsWindow and GetWindowThreadProcessId on foreign windows (reads); SetPreferredAppMode and FlushMenuThemes are process-local.
- [x] Scope items (winId shim, `has_native_window_handles`, `OnceLock`, exact durationFast) — details below.

Focus items:
- winId shim: generated wrapper (`platform_bridge.cxx.cpp:484-491`) is `::std::size_t (*)(::QQuickWindow const &) = ::tw_native_window_handle;`; the shim returns `rust::usize` = `std::size_t`, so declarations match on every OS. On MSVC the `static_cast` is between identical 64-bit types (HWND unchanged). Linux/macOS reasoning sound. `tw_qpa_name` builds the `rust::String` with placement-new in cxx's return slot (cxx owns it); `toStdString()` is UTF-8, so the `noexcept` wrapper can't hit the invalid-UTF-8 throw.
- Ownership guard (`windows.rs:186`): `thread != 0 && owner == std::process::id()`; destroyed or reused handles refused; SAFETY accurate; `watch` and `set_backdrop` call `live_window` first (before `SetWindowLongPtrW` and any DWM call); tests only read the desktop and shell windows.
- Real Windows QPA (target\debug\tinwave-desktop.exe run by the reviewer): default → Mica sidebar (pixel 28,32,41, tinted); `--transparency=off` → solid (29,29,32); `--color-scheme=dark` + Tab+Space → native dark Win32 menu. stderr empty in all three; processes killed.
- OnceLock: success cached, failure not (one leak below).
- Re-flush: only when `os_changed && wanted == FollowSystem`; forced Dark/Light not re-flushed; high contrast counts as following the system; no OS setting touched.
- durationFast: exact, same reader as the app; Theme.qml:324 depends only on `reducedMotion`.

blocking findings:
- tests/platform_qml.rs:89-97 (with src/platform/mod.rs:66) — the changed `developer_switches_change_the_app_in_process_only` is weaker and no longer portable. (1) It fails on Linux/macOS: not `#[cfg(windows)]`, but `"solid: not attached (QPA offscreen)"` only holds on Windows; elsewhere `has_native_window_handles("offscreen")` is true, the bridge attaches, the blocker runs and backdropStatus is `"solid: turned off with --transparency=off"` — it breaks as soon as T-0038/T-0039 compile the bridge. (2) On Windows it no longer proves the switch: the remaining `transparencyEffects == "false"` is `os.transparency_effects && transparency_allowed`, which passes on a machine with transparency effects off even if `setTransparencyAllowed` is never wired. Fix: (a) treat `offscreen` and `minimal` as having no native handles on every OS; (b) pin the switch: read `platform::current().read().unwrap_or_default()` (as the durationFast test does) or run a baseline probe without the switch; assert the baseline transparencyEffects equals `os.transparency_effects`; if transparency is on, assert the switch turns it "false"; if off, say explicitly this machine can't show the switch.

design check: n/a (no QML/UI change); app checked on screen anyway: Mica, solid fallback and dark native menu still work.

non-blocking findings:
- src/platform/windows.rs:460-470 — if `LoadLibraryExW` succeeds but an ordinal is missing, it returns without `FreeLibrary`; every retry (one per OS change message) adds a uxtheme reference; two threads racing the first `menu_api()` can leak one more. SAFETY's "one reference in total" holds only on success. Free the module on the export-miss path.
- windows.rs test `an_os_scheme_message_reflushes_native_menus_that_follow_the_system` (~812-819) — "forced dark: count goes to 4" assumes no high contrast (with HC on, `wanted` stays FollowSystem and it fails); the comment at :833 ("as a fresh process has it") is inaccurate (mode 0 until `Controller::new`). Guard on `os.high_contrast` or document.
- cpp/platform_window.cpp — `tw_qpa_name` goes beyond "winId shim only" for cpp/; needed for the QPA gate, disclosed; add a line to the ticket (ADR 0008 decision 4).

proposals:
- P: `has_native_window_handles` false for offscreen/minimal on every OS now | size: S (fold into the blocking fix)

notes for lead: fix is small. No processes left by the reviewer. Eleven python/python3 processes running (PIDs 60496, 4460, 7560, 23104, 25204, 26308, 35012, 47308, 60272, 66352, 67496), not the reviewer's; several python3.exe look like the hanging launcher. Screenshots/captures in the session scratchpad (delete blocked; not in the repo).

---
Lead (2026-11-03): round 2 = the blocking fix (portable offscreen/minimal rule, P-0217 merged here; pinned switch
assertion) + the FreeLibrary leak + the HC guard and comment in the re-flush test. Lead adds the `tw_qpa_name` scope
note to the ticket.
