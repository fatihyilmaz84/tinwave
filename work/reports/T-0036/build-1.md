HANDOFF T-0036 (round 1)
status: done
branch: t/T-0036-windows-platform-bridge   commit: 7e0c62d (code) + bde9a72 (evidence screenshots only)
summary: A platform layer in src/platform/ (traits, a Qt-free controller holding every decision, a Windows impl that is the only unsafe module, an Unsupported impl elsewhere), exposed to QML through one singleton, PlatformSettings (src/app/platform_bridge.rs). Reduced motion, high contrast and the Mica state are bound into Theme from Main.qml. The sidebar app menu is Popup.Native with the SetPreferredAppMode opt-in, falling back to Popup.Window automatically. Dev switches --accent=#rrggbb and --transparency=off added. Mica, the solid fallback, a non-default accent and the dark native menu verified on screen (Win 11 26200, 150 %).

acceptance criteria:
- [x] Each provider unit-tested; live OS toggles left to the PO runbook — cargo nextest run --workspace: 543 passed / 0 failed / 6 skipped. Controller with a fake platform: 24 tests (platform/controller.rs). Windows providers (platform/windows.rs): the real read-only SettingsReader, build gates, uxtheme ordinals 135/136, the CRT environment, and the message path — synthetic WM_SETTINGCHANGE, WM_THEMECHANGED, WM_SYSCOLORCHANGE and 0x320 sent to a message-only window of their own (never a broadcast); a repeat watch replaces the callback; window destroy removes the hook. Unsupported provider tested; checked with clippy on x86_64-unknown-linux-gnu (aarch64-apple-darwin std not installed). Bridge as QML sees it: tests/platform_qml.rs (4 tests, tests/qml/PlatformProbe.qml, debug only). End-to-end by hand (temporary debug line removed): SendMessage to the app window ran queue → refresh on the Qt thread three times.
- [x] unsafe only in one documented module with per-block SAFETY comments; non-Windows returns Unsupported — platform/windows.rs (`#[allow(unsafe_code)]` on its mod declaration; `undocumented_unsafe_blocks = deny` passes); every provider on non-Windows answers `PlatformError::Unsupported`. The unsafe in src/app/platform_bridge.rs is the ADR 0008 decision-4 bridge exception: one SAFETY-commented raw QQuickWindow pointer deref plus the cxx declarations.
- [x] P-0121 — the app Menu uses `popupType: PlatformSettings.nativeMenus ? Popup.Native : Popup.Window`; platform_qml test checks nativeMenus=true and menuStatus=native; dark native menu screenshot.
- [x] P-0132 — `--accent=#rrggbb` in debug and release, in-process only, validated by `Theme.setAccentOverride` (anything else refused with a console warning); platform_qml tests `developer_switches_…`, `bad_switch_values_…`; screenshots.
- [x] Screenshots light + dark at 150 %, Mica on and solid fallback, non-default accent, dark native menu open.

checks: fmt pass, clippy pass (workspace --all-targets -D warnings), tests 543 passed / 0 failed (6 skipped), deny pass
files changed: apps/tinwave-desktop/Cargo.toml, Cargo.lock (windows crate target dependency, features only), README.md, build.rs (adds platform_bridge.rs); qml/Main.qml, qml/Sidebar.qml, qml/Theme/Theme.qml; src/lib.rs, src/app/mod.rs, src/app/wiring.rs, src/app/platform_bridge.rs (new); src/platform/{mod,controller,unsupported,windows}.rs (new); tests/platform_qml.rs, tests/qml/PlatformProbe.qml (new); 9 PNGs in apps/tinwave-desktop/docs/screenshots/ (second commit)
signals added/changed: none
ui: design-system §2 (Sidebar backdrop, Menus rows), §3 (Theme inputs), §6/§7 motion and high contrast, per ADR 0007 / D-0006. Native FluentWinUI3 Menu (Popup.Native), ToolButton, ItemDelegate; no new Tw*, no new tokens, no raw colours. Windows 11 Pro 26200, 150 %, OS dark mode, transparency on. Screen grabs from a scratch copy of tools/capture.ps1 with desktop crop (PrintWindow can't capture Mica or native menus); the repo script is untouched. Screenshots (apps/tinwave-desktop/docs/screenshots/): t0036-dark-mica-…, t0036-light-mica-… (light app on a dark OS: light Mica), t0036-dark-solid-…, t0036-light-solid-… (--transparency=off), t0036-light-accent-ffb900-mica-…, t0036-dark-accent-1f3a93-mica-…, t0036-dark-native-menu-open-…, t0036-light-native-menu-open-on-dark-os-…, t0036-dark-menu-closed-by-esc-focus-ring-…. Keyboard: Tab → ring on the menu button, Space opens the native menu, Esc closes it, ring stays. Deviations: none; Mica light/dark follows the app scheme, not the OS.
docs updated: apps/tinwave-desktop/README.md (layout table, dev switches, "Platform bridge (Windows)" section). ADR 0007 / design-system §2 wording unchanged (out of scope).

deviations from ticket:
- Theme gets reducedMotion, highContrast and backdropActive (existed) plus setAccentOverride(); transparency is consumed in Rust (decides Mica) and appears in Theme only as backdropActive. PlatformSettings exposes transparencyEffects and backdropStatus for diagnostics/Settings. Theme.qml does not import the bridge (the contrast self-check still runs standalone).
- The menu follows the app's own scheme override: uxtheme AllowDark at start-up, ForceDark/ForceLight when Theme.schemeOverride changes, AllowDark again under high contrast (one extra invokable, setMenuScheme).
- Screenshots committed under apps/tinwave-desktop/docs/screenshots/ (outside scope), own commit.

findings worth knowing:
- Rust's std::env::set_var does not reach Qt on Windows (the MSVC CRT keeps its own environment copy), so QT_QPA_DISABLE_REDIRECTION_SURFACE=1 was ignored and the sidebar rendered solid white (A3). Fix: set it via the CRT's `_putenv_s`; set_backdrop(on) checks that CRT copy and refuses instead of showing white. Tests: crt_environment_round_trips…, a_variable_set_only_in_the_os_copy_…. Before: solid white sidebar; after: Mica in dark and light.
- Offscreen test platform has no HWND: attachWindow records "IsWindow failed" in settingsStatus and prints one stderr line (accepted by tests). On offscreen, setColorScheme(Dark) doesn't take effect, so probe tests don't assert dark.
- Risks: dark menus rely on undocumented uxtheme ordinals 135/136 (documented in platform/windows.rs), gated to build >= 18362, looked up at run time via LoadLibraryEx(SEARCH_SYSTEM32); any miss → Popup.Window. Verified only on 26200. Mica gated to build >= 22621. Non-Windows: nativeMenus false → Popup.Window; macOS (design table: Native) and Linux are T-0038/T-0039.
- Not exercised: real OS toggles and a real Mica text-contrast check (PO runbook A, C, D, E, G, H).
- No Windows settings changed. Leftover python (1) and python3 (8) processes on the machine are not the builder's.

proposals:
- P: Settings-driven scheme/accent/transparency (setSchemeOverride/setAccentOverride/setTransparencyAllowed hooks for a Settings screen) | size: S
- P: QML probe coverage of Main.qml bindings on other OSes when their bridges get impls | size: S
- P: Repo capture mode for desktop crops (Mica and native menus need a screen grab; PrintWindow can't) | size: S
- P: Design-system §2/§3 text for the transparency/high-contrast backdrop blockers and menu-follows-app-scheme (ADR 0007 A3 listed high contrast as proposed/untested; now implemented) | size: S

questions for lead:
- Keep the screenshot commit bde9a72, or store the evidence under work/reports/T-0036/?
- Is Popup.Window acceptable on macOS/Linux until T-0038/T-0039, or should Unsupported map to Popup.Native there?

---
Lead (2026-11-03):
- Keep bde9a72: docs/screenshots is where UI evidence lives (as T-0035/T-0043/T-0050).
- Popup.Window on macOS/Linux until T-0038/T-0039 verify Native there: accepted (D-0001 Windows first); noted on
  T-0038 and T-0039.
- Proposals: P-0187 (Settings-driven hooks) parked → Settings screen ticket; P-0188 (other-OS probe coverage) parked →
  T-0038/T-0039; P-0189 (desktop-crop capture mode) accepted → T-0052 with P-0175 and P-0186 (capture.ps1
  improvements); P-0190 (design-system §2/§3 text) → lead applies on merge.
