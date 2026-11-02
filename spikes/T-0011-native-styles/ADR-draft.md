# ADR 0003 amendment (draft) — native-style findings from spike T-0011

- **Status:** Proposed (amends ADR 0003; does not supersede it)
- **Date:** 2026-11-02
- **Evidence:** `spikes/T-0011-native-styles/` (prototype host, `Theme.qml`, raw results, screenshots).
  Measured on Windows 11 Pro 26200, Ryzen 7 7800X3D, RTX 4070, 2560x1440 at 150 %, **Qt 6.12.0 MSVC** (the only
  kit installed; the 6.8 floor itself was not run, see "Qt floor"). macOS and Linux were not runnable here.

## Context

ADR 0003 chose native Qt Quick Controls styles per OS, native menus/dialogs, system font/accent/colour scheme, a
token-driven `Theme.qml`, Qt 6.8 LTS as the floor and Mica/vibrancy behind a `WindowEffects` trait. Spike T-0011
asked whether Qt gives that look and OS integration at acceptable memory cost and what the minimal correct
`Theme.qml` is.

**Verdict: ADR 0003 is confirmed for Windows with four amendments (A1-A4) and several corrections to
design-system.md. Nothing found requires abandoning native-first.**

## Decision

**Confirmed (Windows, measured)**

1. `FluentWinUI3` is the Windows style. It follows light/dark and the OS accent through the palette, renders every
   control we need (Button, TextField, ComboBox, Switch, CheckBox, Slider, SpinBox, TabBar, ProgressBar, Menu,
   ItemDelegate), and keeps working under an in-app `setColorScheme` override.
2. System title bar and caption buttons: Qt's window frame follows the scheme (dark title bar when the scheme is dark,
   also after `setColorScheme`).
3. Native file/folder dialogs (`FileDialog`, `FolderDialog`): real Win32 `#32770` item dialogs, localized, OS dark mode.
4. Mica is viable behind the `WindowEffects` trait with a solid fallback, at no measurable cost (details A3).
5. Live OS scheme/accent: `Application.styleHints.colorScheme` and `SystemPalette.accent` (`palette.accent`) are
   readable, notify, and `setColorScheme(Light|Dark|Unknown)` overrides/reset works live (native palette, tokens,
   accent shade and title bar all follow within one event-loop turn).

**Amendments**

- **A1. Select the style explicitly at startup.** `QQuickStyle::setStyle("FluentWinUI3")` before the engine exists
  (`QT_QUICK_CONTROLS_STYLE` stays as a dev override). Qt 6.12's default on Windows is the `Windows` style, which has
  **no dark mode** (its palette stays `#F0F0F0`/black while the scheme is dark: unreadable controls, see
  `screenshots/style-windows-dark.png`) and ignores `setColorScheme`. `QtQuick.Controls.Native` is a *compile-time*
  import added in 6.12, not a runtime style name, so it is not usable under the 6.8 floor. FluentWinUI3 falls back to
  Fusion for SplitView, StackView, Drawer, HeaderViews, Dial, Tumbler and SwipeView/Delegate (6.12 docs list), so do
  not build on those without checking.
- **A2. Native menus on Windows need a dark-mode opt-in, or we accept `Popup.Window`.** `Popup.Native` works
  (Win32 `#32768`, no fallback; `popup()` blocks until dismissed; no icons/delegate/`cascade`), **but it is drawn
  light in a dark OS and app** (`screenshots/menu-native-win32-default-light-in-dark-app.png`). Calling the
  *undocumented* `uxtheme.dll` ordinal 135 (`SetPreferredAppMode`, 1 = AllowDark) + 136 (`FlushMenuThemes`) in our own
  process before the first menu makes it follow the OS mode (`screenshots/menu-native-win32-with-SetPreferredAppMode-dark.png`).
  Options for the PO: **(a)** keep `Popup.Native` and call that API from `apps/tinwave-desktop/windows.rs`
  (recommended: smallest visual gap, per-process, no setting changed, widely used; risk: undocumented, could change);
  **(b)** `popupType: Popup.Window` with the FluentWinUI3 menu (dark-correct, WinUI-looking, not OS-drawn);
  **(c)** native and light in dark mode (rejected). Linux: Qt docs say `Popup.Native` only exists as a stand-alone
  context menu with the GTK3 platform theme; elsewhere (KDE etc.) it falls back and Fusion draws it, which is the
  design table's "else Fusion".
- **A3. Mica works only with these four things together**, otherwise the sidebar renders **solid white**:
  `QQuickWindow::setDefaultAlphaBuffer(true)` + transparent window colour, `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE = 2)`,
  `DwmExtendFrameIntoClientArea(-1,-1,-1,-1)`, **and `QT_QPA_DISABLE_REDIRECTION_SURFACE=1` set before the
  `QGuiApplication` is created** (adds `WS_EX_NOREDIRECTIONBITMAP`; without it the HWND's white redirection surface
  shows through every transparent pixel). The env var is per process, so it also applies to the mini-player window.
  `WindowEffects::apply` should return `Unsupported(reason)` when: build < 22621 (Win11 22H2; implemented), "Transparency effects"
  is off (registry `EnableTransparency = 0`; implemented; DWM would paint its own solid colour, not our `sidebar` token,
  not yet observed), or high contrast is on (proposed, untested). Cost (9 runs each, FluentWinUI3 controls page): first frame 468 ms to 483 ms median (+15 ms, inside the
  run-to-run spread of 446-531 ms), working set 111.1 to 111.8 MB, idle CPU 15 to 16 ms per 1.5 s. GPU time and
  battery were **not** measured. Text contrast against the live Mica material is unchecked (token checks use the
  solid `sidebar`).
- **A4. Confirmations on Windows and Linux are a Qt Quick `MessageDialog`, not a native one.** Probe: window class
  `Qt6120QWindow`, drawn with the active style (Fluent look, system title bar, localized buttons). Qt docs: native
  `MessageDialog` only on Android, iOS, macOS. Accept it (it is themed and correct) and change the design table row;
  a Win32 `TaskDialog` call from the bridge would be a custom per-OS integration and is not recommended now.

**Theme corrections (design-system.md §3, needs a design ticket per §9)**

1. **`base` font.** Spec: `Qt.application.font.pixelSize`. Measured: that is Segoe UI 9 pt = **12 px**, while
   FluentWinUI3 controls use Segoe UI Variable 10.5 pt = **14 px** (the spec's "Windows 14"); Basic/Fusion/Windows 12,
   Material 14, Universal 15. `Theme.base` must come from the active style's own control font (a hidden `Label`),
   not from `Qt.application.font`. QML already derives `pixelSize` from `pointSize`, so no DPI maths is needed.
2. **New token `accentFill`.** FluentWinUI3 hard-codes the label colour on accented controls (white in light, black in
   dark; it ignores `palette.highlightedText`). Qt reports a different shade of the OS accent per scheme (here `#8E3AA7` light,
   `#DB9EE5` dark; both pass 4.5:1 against the style's fixed label), so the OS accent we saw is fine, but any other accent (Tin Teal fallback in dark, a user override, a pale
   yellow in light) gives unreadable labels (`screenshots/fluent-light-yellow-NOFIX.png`, `fluent-dark-navy-NOFIX.png`).
   `accentFill` = accent moved toward black/white until it reaches 4.5:1 against that fixed label; feed it to
   `palette.accent` on the window. Verified for 15 accents x 2 schemes (`results/selftest-contrast.txt`).
3. **`onAccent`** cannot be a fixed colour for arbitrary accents; it is computed (default, white, ink, black: first that
   reaches 4.5:1).
4. **`accentText`** (spec rule: darken/lighten until >= 4.5:1) works: straight sRGB mix toward black/white, binary search,
   quantised to 8 bit, checked against every surface text can sit on (window, sidebar, content, raised, selected).
   Results for 15 accents in both schemes are all >= 4.5:1 (min 4.51). Hue is kept; pale yellow becomes brown-gold
   (`#FFB900` to `#8B6400` in light).
5. **Spec bugs found by the self-test (so "all text pairs >= 4.5:1 (checked)" is currently false):** light `textFaint`
   `#6E6E77` is 4.28:1 on `sidebar` and 4.39:1 on the Tin Teal `selected` tint; light `srcDownload` `#2E7D46` is
   4.30:1 on `sidebar`. Suggested: `textFaint` `#6A6A73` (min 4.54), `srcDownload` `#2B7842` (min 4.60).
6. **`selected`** percentages do not match the hexes: `#E6F1F2` is about 10 % of Tin Teal over white (spec says 12 %),
   `#1B2B2D` is about 12 % of `#5CC6CF` over `#161618` (spec says 18 %). The prototype uses 10 %/12 %; pick one rule.
7. **Alpha hex trap.** `hover` `#0000000A` and `separator` `#0000001A` are CSS `#RRGGBBAA`; QML parses `#AARRGGBB`, so
   copying them verbatim gives alpha 0. Tokens must be `Qt.rgba(0,0,0,0x0A/255)` (documented in `Theme.qml`).
8. **Accent fallback.** There is no `QStyleHints::accentColor` in Qt 6.12 (checked at runtime and in headers), so QML
   cannot tell "OS exposes an accent" from "Qt synthesised one" (`QPalette::Accent` defaults to Highlight). The bridge
   must supply `osAccentAvailable` (XDG portal / platform theme on Linux); default per OS in `Theme.qml`.
9. **Reduced motion and high contrast need the bridge on Qt 6.8.** `Application.styleHints.accessibility`
   (`contrastPreference`) exists from **6.10**, `motionPreference` from **6.12** (Linux portal `reduced-motion` too).
   Until the floor is >= 6.12 read them from Win32: `SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)` and
   `SPI_GETHIGHCONTRAST`, and re-read on `WM_SETTINGCHANGE` via a native event filter (prototype `PlatformInfo`,
   to become a `windows.rs` provider). Plumbing verified with a message sent to our own window; a real OS toggle is
   unverified (see `PO-RUNBOOK.md`). A message *posted* with `PostMessage` never reached Qt's filter; use `SendMessage` in tests.
10. Elevation: no token needed (popups are native); `iconSize*`, `focusRing*` and `hitTarget*` tokens were added from §3.3/§3.4/§7.

**Qt floor (6.8 LTS): keep provisionally, verify before it is relied on.** Everything above was measured on 6.12.
Docs/changelogs say these are **newer than 6.8**: `QAccessibilityHints` / `contrastPreference` (6.10),
`motionPreference` (6.12), `SearchField` (6.10; use `TextField` + icon on 6.8), improved FluentWinUI3 contrast-theme
support (6.10), `DoubleSpinBox` (6.11), `ContextMenu` attached type (6.9), `Qt::ExpandedClientAreaHint` /
`NoTitleBarBackgroundHint` (6.9; needed for the macOS "full-size content" design on a clean API), Wayland
server-side-decoration preference (6.11), `Menu.separatorsCollapsible`, `ComboBox.highlightOnHover`, shortcut text in
`MenuItem` (6.12), compile-time `QtQuick.Controls.Native` (6.12). Introduced **by** 6.8 per docs: FluentWinUI3,
`Popup.popupType` / native `Menu` (Windows, macOS), `QStyleHints::colorScheme` setter. **Not verifiable here:** the
high-contrast look of native controls on 6.8 (Fluent only got proper support in 6.10). Recommendation: re-run this
harness on a 6.8.x kit (it builds there: the 6.10/6.12 hints are `QT_VERSION`-guarded in C++), and let the PO decide
whether the Windows accessibility story justifies a 6.10 floor. This is an ADR + PO decision, not made here.

**Memory (spec §12 "idle 50-100 MB")**, working set 1.5 s after first frame, medians of 9, Release, D3D11:

| Config | minimal window WS / private | controls page WS / private | first frame (page) |
|---|---|---|---|
| Basic | 72.6 / 18.7 MB | 92.4 / 33.4 MB | 424 ms |
| Fusion | 73.3 / 19.2 | 94.0 / 34.9 | 425 ms |
| Windows (Qt default) | 73.5 / 19.1 | 96.4 / 36.3 | 434 ms |
| **FluentWinUI3** | 84.3 / 29.2 | 111.1 / 49.7 | 468 ms |
| FluentWinUI3 + Mica | n/a | 111.8 / 50.3 | 483 ms |

FluentWinUI3 costs about **+16 MB private (+19 MB working set)** on the controls page over Basic (+10 MB on an empty
window: its generated `Config` singleton tree and image assets) and about +40 ms startup. Qt Quick itself is already
about 72 MB working set (mostly shared Qt and GPU-driver pages) before any Tinwave code, so §12 must say which counter
the 50-100 MB targets use (recommend private working set). Disk: FluentWinUI3 QML+images 3.7 MB + 3.6 MB DLL, Fusion 0.56 + 2.1, Basic 0.57 + 2.5.
Startup differences between styles are within the run-to-run spread on a loaded machine; memory differences are stable.

## Consequences

- Windows UI keeps the OS accent/scheme with little custom QML; the cost is one extra token (`accentFill`), one
  corrected token source (`base`), one bridge provider (reduced motion, high contrast, transparency, scheme-aware
  menu mode) and one process-wide env var for Mica.
- design-system.md §2 and §3 must be edited (table below, token fixes above) in the same change that accepts this ADR.
- Real OS accent/theme/reduced-motion/high-contrast/transparency switching, 6.8 behaviour and everything on macOS and
  Linux remain **unverified** (D-0001 real machines).
- No signal is involved: scheme, accent, reduced motion and high contrast are UI settings, not catalog signals. They
  must not feed the context engine without a signal-catalog entry and privacy tier.

## Spec / docs updated

To change when accepted (not changed by the spike): `docs/design/design-system.md` §2 (table below), §3.1 (token
values/rules 2-7), §3.2 (`base`), §3.3/§7 (bridge-supplied reduced motion/high contrast), ADR 0003 (point to this
amendment), spec §2 (Qt floor wording) and §12 (memory counter), `docs/rules/qml-ui.md` (explicit style selection,
`palette.accent: Theme.accentFill`).

---

## Appendix A — design-system.md §2 rows

Legend: **confirmed** = observed working as written; **workaround** = works with the stated extra step;
**unsupported** = does not work as written; *unverified* = not runnable here (docs cited).

| Element | Windows 11 (measured) | macOS | Linux |
|---|---|---|---|
| Controls style | **confirmed** FluentWinUI3 (explicit `setStyle`, A1) | *unverified* `macOS` style runs only on macOS ("module macOS is not installed" on Windows) | **confirmed** Fusion runs and follows `setColorScheme` (palette + accent); platform palette (Breeze/Adwaita) *unverified* |
| Window chrome | **confirmed** system title bar, dark/light follows scheme | *unverified*; full-size content wants Qt >= 6.9 hints | *unverified*; Wayland SSD preference only from 6.11, so 6.8 may get client-side decorations |
| Sidebar backdrop | **workaround** Mica needs A3 (env var + alpha + DWM attrs), solid fallback conditions | *unverified* vibrancy | **confirmed** solid token (no OS dependency) |
| Menu bar | **confirmed** none; app-menu `Menu` from a button | *unverified*; Qt 6.8 macOS style creates native menubars (release notes) | **confirmed** none |
| Menus / context menus | **workaround** `Popup.Native` works but light-in-dark without A2 | *unverified* (docs: native) | **confirmed by docs** native only via GTK3 platform theme, else Fusion-drawn |
| File / folder dialogs | **confirmed** native `#32770` | *unverified* (docs: native) | *unverified* XDG portal needs the `xdgdesktopportal` platform theme |
| Confirmations | **unsupported as written** Qt Quick dialog, style-themed (A4) | *unverified* (docs: native) | **unsupported as written** same fallback (docs) |
| Font | **workaround** controls use Segoe UI Variable 14 px; `Qt.application.font` is Segoe UI 12 px, so read `base` from the style (Theme fix 1) | *unverified* | Fusion uses the application font (12 px on this machine); desktop setting *unverified* |
| Light / dark | **confirmed** follow + override + reset live in-app; real OS toggle *unverified* | *unverified* | *unverified* (Qt 6.12 reads portal colour-scheme per docs) |
| Accent | **confirmed** `palette.accent`, scheme-dependent shade; **workaround** `accentFill` for contrast; real OS change *unverified* | *unverified* | **workaround** no "is exposed" API (Theme fix 8) |
| Media keys / OS overlay | not examined (out of scope) | not examined | not examined |
| Background presence | not examined | not examined | not examined |
| Shortcuts | not examined | not examined | not examined |
| Notifications | not examined | not examined | not examined |

Also checked, not a §2 row: reduced motion and high contrast detection (Win32 read: **confirmed**, both false on
this machine; Qt 6.12 hints agree: `contrastPreference=0`, `motionPreference=0`).

## Appendix B — licenses of what the prototype uses

| Component | License | Concern |
|---|---|---|
| Qt Quick, Qt Quick Controls 2 (Basic, Fusion, FluentWinUI3, Windows), Qt Qml, Qt Gui | Qt Commercial OR LGPL-3.0 OR GPL-2.0/3.0 (SPDX headers of the installed QML files) | LGPL-3: dynamic linking only, ship Qt as DLLs/shared libs and licence notices. Same terms as the rest of Qt already in ADR/spec. |
| Qt Quick Dialogs (`QtQuick.Dialogs`, FileDialog, FolderDialog, MessageDialog) | Commercial OR LGPL-3.0 OR GPL-2.0 (Qt docs "Qt Quick Dialogs") | none beyond the above; the native dialogs are OS code |
| FluentWinUI3 image assets | no separate attribution file in qtdeclarative `src/quickcontrols/fluentwinui3`; treat as Qt's licence | check with `qtattributionsscanner` at packaging; designed after Microsoft's Fluent kit but not a Microsoft licence we must carry |
| Qt Labs modules (`Qt.labs.platform` etc.) | not used | would be tech-preview; avoid |
| Windows APIs (DWM, SystemParametersInfo, uxtheme ordinals 135/136) | OS | ordinals 135/136 are **undocumented** (A2) |
| Lucide icons (design-system §3.4, not used in the spike) | ISC | none |

## Appendix C — open items / follow-ups

1. PO runs `PO-RUNBOOK.md` (real accent/theme/reduced motion/high contrast/transparency/text size/menu dark mode).
2. Re-run the harness on a Qt 6.8.x MSVC kit.
3. macOS and Linux runs on real machines (style, menus, dialogs, vibrancy, portal accent/scheme).
4. Design ticket: token fixes 1-7 in design-system.md.
5. Windows bridge provider (`windows.rs`): reduced motion, high contrast, transparency, scheme-aware menu mode, Mica.
