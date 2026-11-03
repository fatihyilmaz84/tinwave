REVIEW T-0053 (round 2)
verdict: APPROVE
checks (run by reviewer): fmt pass; clippy pass (workspace, all targets, -D warnings; cached clean); tests pass (cargo nextest run --workspace: 582 run, 582 passed, 6 skipped); deny pass
merge-tree: `git merge-tree --write-tree main t/T-0053-platform-bridge-followups` → 905a26a0…, no conflicts. HEAD 1aa919f, clean.

acceptance criteria:
- [x] Ownership guard and re-flush tests — round-1 tests pass; `windows::tests::an_os_scheme_message_reflushes_native_menus_that_follow_the_system` has the high-contrast guard (windows.rs:869-879); comment at :880 correct (the controller sets FollowSystem at start-up).
- [x] Offscreen "not attached (QPA offscreen)" — in `has_native_window_handles` (mod.rs:66-74) `offscreen` and `minimal` return false before the `cfg!(windows)` arm (every OS); `platform::tests::only_the_windows_plugin_hands_out_hwnds_on_windows` checks both unconditionally; `developer_switches_change_the_app_in_process_only` portable; `windows_reads_the_os_and_offers_native_menus` passes.
- [x] fmt, clippy, tests, deny; no Windows setting changed (registry only read: EnableTransparency=1, AppsUseLightTheme=0; SetPreferredAppMode and FlushMenuThemes process-local).
- [x] Round-1 blocker fixed: the switch test is pinned (platform_qml.rs:100-130).

Focus items:
1. Guard is the first branch. Real app (target\debug\tinwave-desktop.exe): `--color-scheme=dark` → tinted Mica sidebar (26,33,42 vs content 29,29,32), Tab+Space → native dark Win32 menu ("Tinwave 0.1.0", "Quit"); `--transparency=off` → solid (29,29,32). stderr empty; processes killed and confirmed gone.
2. `the_transparency_switch_is_what_turns_transparency_off`: baseline vs `platform::current().read().unwrap_or_default()` (off Windows the default — transparency true — matches the app's assumption: portable). Transparency on here: ~0.9 s, three probes, no NOTE (switch path ran); explicit NOTE when off. Mutation re-run (scratch `git archive` copy, own CARGO_TARGET_DIR): `set_transparency_allowed(true || allowed)` fails this test ("the switch turns it off", left "true", right "false") and `developer_switches_change_the_app_in_process_only`.
3. `resolve_menu_api` frees on miss (windows.rs:480-488), keeps no pointer into it. The `RESOLVING` mutex guards two threads racing the first resolution (a second module reference); no re-entry possible (nothing inside takes another lock or calls back into `menu_api`; LoadLibraryExW on already-loaded uxtheme runs no DllMain) — a leaf lock, no deadlock risk even though `menu_api` is reached under the controller mutex. Failure still not cached; each retry releases its reference. The shfolder test checks shfolder.dll isn't loaded beforehand (fails loudly otherwise); passes in a shared-process `cargo test -p tinwave-desktop --lib platform` run (50 passed); the `let _ = uxtheme;` mutation fails it.
4. HC guard and comment correct; earlier FollowSystem re-flush assertions hold in both cases.

blocking findings:
- none

design check: n/a (no QML/UI change); app checked on screen anyway.

non-blocking findings:
- windows.rs:462-468 — SAFETY says "uxtheme is already loaded by Qt" (not true on the shfolder test path); error strings say "uxtheme.dll" whatever `dll` is. Wording only.
- windows.rs:461 — `resolve_menu_api(dll: PCWSTR)` is a safe fn relying on a valid NUL-terminated pointer; private and both callers pass `w!` literals — acceptable; `&'static` wide string would be tighter.

proposals:
- none

notes for lead: the literal NUL in `crt_environment_round_trips_and_is_also_visible_to_the_os` (`set_crt_env("BAD<NUL>NAME", "1")`, byte 27032, the only NUL; from T-0036 7e0c62d) can become `"BAD\0NAME"` — same string (Rust's `\0` has no octal continuation), test meaning unchanged (CString rejects the interior NUL; Err); checked in scratch (0 NULs, test passes). Because of that byte, ripgrep treats windows.rs as binary. Scratch copy deleted; nothing running; screenshots r2-mica.png, r2-menu.png in the scratchpad.

---
Lead (2026-11-03): merged 78aa25b; NUL byte replaced with `\0` by the lead on main (0 NULs left). The two wording nits
are left for the next change to windows.rs.
