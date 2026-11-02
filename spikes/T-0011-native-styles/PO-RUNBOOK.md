# PO runbook — what only a human can verify (T-0011)

The agent run never changed a Windows setting (no accent, light/dark, high contrast, reduced motion, transparency
or theme change). It used in-app overrides instead:

- light/dark: `Theme.setSchemeOverride()` → `QStyleHints::setColorScheme` (works, see ADR-draft);
- "different accent": `Theme.accentOverride` + `palette.accent` on the window;
- reduced motion / high contrast: **read only** (`SystemParametersInfo`), plus a plumbing test that sent a
  `WM_SETTINGCHANGE` message to the prototype's own window (not broadcast).

**Unverified until you run the steps below:** that a *real* OS accent / theme / reduced-motion / high-contrast /
transparency change reaches Qt and the prototype live. Everything here is read-only for the app; you change the
Windows settings, then change them back.

## 0. Build once (≈ 2 min)

```powershell
cd D:\dev\Tinwave-worktrees\T-0011-spike-native-qt-styles\spikes\T-0011-native-styles   # or wherever the branch is
.\build.ps1          # needs VS 2026 + Qt 6.12 MSVC + Ninja (same as the dev machine); output .\build\t0011-host.exe
```

## 1. The live window

```powershell
.\live.ps1                       # FluentWinUI3, window stays open 180 s, then closes itself
.\live.ps1 -Seconds 300 -HostArgs "--mica"   # same, with Mica behind the sidebar
```

The page has a **"Live OS state"** line at the top (colour scheme, `palette.accent`, reducedMotion, highContrast,
transparencyEffects, Qt's own hints, app font size, and a counter "OS setting events seen"). The console prints an
`EVENT …` line each time Windows tells the window that a setting changed. Keep the window visible while you change
settings in the Windows Settings app next to it. **Copy the console output into the ticket after each test.**

## 2. Tests (each is: do → expected → note what you see)

Write down your current values first so you can restore them: Accent colour, mode (Light/Dark), Animation effects,
Transparency effects, Contrast theme (None), Text size.

| # | Do (Windows Settings) | Expected | Why it matters |
|---|---|---|---|
| A | Personalization → Colors → **Accent color**: pick 4–5 very different colours while the window is open: a light yellow/gold, a pink, a dark blue, a green, then your own. | Within about a second `palette.accent=` changes and the Switch, Slider, CheckBox, tab underline and "Primary" button recolour. The "Primary" button label stays readable (the prototype feeds `Theme.accentFill` to the native controls). The line `raw accent … -> accentText …` stays ≥ 4.5:1. An `EVENT msg=0x001a …` or `0x0320` line appears. | Proves `palette.accent` is live, and that the contrast fix works on real OS accents (the agent only simulated accents). Report which accents made the Primary button hard to read **without** the fix: start the window with `-HostArgs "--no-accent-fix"`. |
| B | Personalization → Colors → **Choose your mode**: Light, then Dark, then back. | `colorScheme=` flips, title bar flips, all tokens flip, no restart needed. | Proves live light/dark. (In-app override already works: `.\live.ps1 -HostArgs "--scheme","light"`.) |
| C | Accessibility → Visual effects → **Animation effects** Off, then On. | `reducedMotion=true`, `durationFast=0`, event counter increments; back to `false`, `120`. `Qt hints … motionPreference=` shows what Qt 6.12 reports (informational; it needs Qt ≥ 6.12). | Proves the Win32 read + live update that Theme motion tokens need on Qt 6.8–6.11 (which have no motion hint). |
| D | Accessibility → **Contrast themes**: pick *Aquatic*, then *Night sky*, then None. (Shortcut: left Alt + left Shift + PrtScn.) | `highContrast=true (<theme name>)`, Tw parts (sidebar, rows, texts) switch to system colours, `Qt hints … contrastPreference=` becomes non-zero (Qt ≥ 6.10). Check the native Fluent controls look correct and legible. | Design-system §7 requires high contrast support. Qt 6.10 notes say FluentWinUI3 got better contrast-theme support, so Qt 6.8 may look worse here. |
| E | Personalization → Colors → **Transparency effects** Off, then run `.\live.ps1 -HostArgs "--mica"`. | Console: `mica=skipped:transparency effects off in OS`, sidebar is the solid `sidebar` token. Then switch it back On with the window open (and with `--mica` on) and note whether the sidebar changes to a different grey while off. | If DWM draws its own solid colour (not our token) while transparency is off, text contrast on the sidebar is unchecked in that state. |
| F | Accessibility → **Text size**: drag the slider to ~150 %, restart the window (and once with it open). | Read `appFont=` in the Live line and compare the visible text size. Report whether it changes live, only after restart, or not at all. | Design-system §7 says "respect text scaling". Qt may not follow this setting. |
| G | With `--mica` open, click on another window so the prototype is inactive, then back. | Note whether the sidebar tint changes when inactive (Mica's inactive look) and whether text stays readable. | Mica is an OS material; contrast against it is only guaranteed for the solid fallback. |
| H | Dark OS mode: `.\live.ps1 -HostArgs "--probe","menu"` (closes itself after the menu is shown and dismissed; you will see it flash). Then again with `"--probe","menu","--dark-menus","1"`. Repeat in Light mode. | Without `--dark-menus`: the native menu is **light** in a dark OS (agent screenshot `screenshots/menu-native-win32-default-light-in-dark-app.png`). With `--dark-menus 1` it should follow the OS mode. | Decides whether we adopt the undocumented `SetPreferredAppMode` call (see ADR-draft options). |

## 3. After the tests

Restore: accent colour, mode, animation effects, transparency effects, contrast theme (None), text size.
Nothing else was modified by the prototype (no registry writes, no files outside this folder; `--dark-menus` only
affects the prototype's own process).

## 4. What to report back (paste into the ticket)

- For A: accents you tried, the `palette.accent=` value shown for each in light and dark, and whether the Primary
  button label was readable with and without `--no-accent-fix`.
- For C/D/E/F: the console `EVENT` lines and the Live line values before/after.
- Any crash, hang or window that did not close by itself (it should always close within the `-Seconds` value + 10 s;
  a watchdog kills it otherwise).
