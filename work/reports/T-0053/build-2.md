HANDOFF T-0053 (round 2)
status: done
branch: t/T-0053-platform-bridge-followups   commit: 1aa919f
summary: Blocking finding fixed: `offscreen` and `minimal` never count as native-handle plugins on any OS, and the transparency switch is pinned against a no-switch baseline instead of a bare `false`. The uxtheme module is released when an ordinal lookup misses, and the first resolution is serialised. The re-flush test no longer breaks under high contrast; its comment is corrected.

acceptance criteria (round 1 items still hold; round 2 items):
- [x] Portable offscreen/minimal rule (P-0217) — `has_native_window_handles` returns false for `offscreen` and `minimal` before any per-OS arm; `platform::tests::only_the_windows_plugin_hands_out_hwnds_on_windows` asserts both on every OS.
- [x] Switch pinned — new `platform_qml::the_transparency_switch_is_what_turns_transparency_off`: reads the OS (`platform::current().read().unwrap_or_default()`), runs a baseline probe with no switch (baseline `transparencyEffects` must equal `os.transparency_effects`); if the OS has transparency on, `--transparency=off` must give "false" and `--transparency=system` "true"; if off, prints an explicit NOTE and returns. `developer_switches_change_the_app_in_process_only` keeps `solid: not attached (QPA offscreen)` (now portable).
- [x] uxtheme freed on the export-miss path — `resolve_menu_api(dll)` calls `FreeLibrary` on a miss; SAFETY describes success (one kept reference) and miss (released); `menu_api` takes a `RESOLVING` mutex around the first resolution; new test `a_dll_without_the_ordinals_is_released_and_reported` loads `shfolder.dll` through the resolver, asserts the `export 135` error and that `GetModuleHandleW` finds the module not loaded afterwards.
- [x] Re-flush test guarded — the forced-dark step runs only when `platform.read().high_contrast` is false (else prints a NOTE and skips); final comment: "back to following the system, the mode the controller set at start-up".

checks: fmt pass, clippy pass, tests 582 passed / 0 failed (6 skipped), deny pass
files changed: apps/tinwave-desktop/src/platform/mod.rs, src/platform/windows.rs, tests/platform_qml.rs
signals added/changed: none
ui: none
docs updated: doc comments in the touched files
deviations from ticket: none

Revert tests (restored): `FreeLibrary(uxtheme)` → `let _ = uxtheme;` fails `a_dll_without_the_ordinals_is_released_and_reported`; forcing `set_transparency_allowed(true || allowed)` fails both switch tests (this machine has transparency on, so the pinned path ran); removing the offscreen/minimal guard fails the mod.rs test only after also forcing the non-Windows arm (on Windows the `qpa == "windows"` arm already refuses offscreen); the high-contrast guard can't be revert-tested here (needs an HC theme; no Windows settings changed).

Notes: `windows.rs` contains a literal NUL byte inside the `set_crt_env("BAD<NUL>NAME", ...)` test string (pre-existing, left). `FreeLibrary` is in the already-enabled Foundation feature; Cargo.toml unchanged. No processes left.

proposals: none new
questions for lead: none

---
Lead (2026-11-03): the literal NUL byte in source is a hazard (like the BEL byte in T-0026) — replace it with the `\0`
escape on merge if the reviewer confirms nothing else depends on it.
