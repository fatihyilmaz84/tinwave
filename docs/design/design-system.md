# Tinwave design system — "Quiet Native"

Status: v0.1 · Decision: [ADR 0003](../adr/0003-native-first-design-system.md) · Spec: §14 ·
Rules: [qml-ui.md](../rules/qml-ui.md)

Visual reference (PO's design canvas): <https://claude.ai/artifact/YDnwECyurREDCZhXPtfFPH> —
Foundations, Components, Native per platform, and four key screens (Library · Windows, Album + Queue · macOS,
Smart Radio · Linux, Signal Center · Windows dark). A snapshot of its source is kept in
[`canvas/`](canvas/) (`*.dc.html` artboards + `canvas.json` layout; agents can read these even without access
to the canvas). **This file is the source of truth**; the canvas illustrates it.
If they disagree, this file wins and the canvas gets fixed.

---

## 1. Principles

1. **Native first.** The OS draws window chrome, menus, dialogs, scrollbars, text fields, buttons, switches,
   combo boxes and sliders (Qt Quick Controls native styles). Tinwave draws only music-specific parts (`Tw*`).
2. **Artwork is the color.** Neutral surfaces, one accent. Covers are the brightest thing on screen.
3. **Calm by default.** Everyday listening is one click away; advanced controls sit one level deeper.
4. **Source always visible.** Local / server / downloaded is shown wherever a track can play — icon *and* word.
5. **Every touch is a signal.** Each component lists the catalog signals it emits, with input source.
   A component without its signals is not done.

## 2. Platform adaptation

One QML tree. The controls style is selected at startup from the OS; layout and behavior are identical.

| Element | Windows 11 | macOS | Linux |
|---|---|---|---|
| Qt Quick Controls style | `FluentWinUI3`, selected explicitly at start-up (Qt's default `Windows` style has no dark mode) | `macOS` | `Fusion` + platform palette (Breeze/Adwaita colors via the platform theme) |
| Window chrome | System title bar + caption buttons | System title bar, traffic lights, full-size content | Server-side decorations (client-side only if the compositor forces it) |
| Sidebar backdrop | Mica where supported (needs `QT_QPA_DISABLE_REDIRECTION_SURFACE=1`), else solid `sidebar` | Vibrancy where supported, else solid | Solid `sidebar` |
| Menu bar | None visible; app-menu button in sidebar header | Global native menu bar (required) | None visible; app-menu button |
| Menus / context menus | `popupType: Popup.Native` + dark-mode opt-in (`SetPreferredAppMode`, ADR 0007); `Popup.Window` if that fails | Native | Native where supported, else Fusion |
| File / folder dialogs | `QtQuick.Dialogs` native | Native | XDG portal |
| Confirmations | Qt Quick `MessageDialog` (FluentWinUI3-themed; Qt has no native one here) | Native alert/sheet | Qt Quick `MessageDialog` (Fusion-themed) |
| Font | Segoe UI Variable (system) | SF Pro (system) | Desktop font setting |
| Light / dark | Follow system, override in Settings | Follow Appearance, override | Follow portal `color-scheme`, override |
| Accent | OS accent | OS accent | Desktop accent if exposed, else Tin Teal |
| Media keys & OS overlay | SMTC | MPNowPlayingInfoCenter + MPRemoteCommandCenter | MPRIS (D-Bus) |
| Background presence | Optional tray icon with transport menu | Dock menu | Optional tray (StatusNotifierItem) |
| Shortcuts | Ctrl-based | Cmd-based | Ctrl-based |
| Notifications | Off by default, native | Off by default, native | Off by default, freedesktop |

Rules:
- Minimum Qt is **6.10** (FluentWinUI3 with contrast themes, native Qt Quick menus, `QStyleHints::setColorScheme`,
  `QPalette::Accent`, `QAccessibilityHints`, `SearchField`). Measured on 6.12 by spike T-0011; ADR 0007, D-0006.
- **Never restyle a native control** to look like another platform or like a web control. You may set size,
  `palette` roles from tokens, and layout — not backgrounds/contentItems of native controls.
- Window effects (Mica, vibrancy) sit behind a `WindowEffects` trait in `apps/tinwave-desktop`
  (`windows.rs` / `macos.rs` / `linux.rs`); unsupported returns `Unsupported` and the solid token is used.
  Media-session integrations live in `tinwave-playback` behind its own trait (architecture.md boundaries).
- Every input path (mouse, keyboard, media key/OS overlay, tray, dock, mini-player, remote) calls the same
  view-model method with its `InputSource`.

## 3. Tokens (`Theme.qml`)

QML never contains a raw color, font size, spacing, radius or duration. Everything comes from the
`Theme` singleton (`apps/tinwave-desktop/qml/Theme/Theme.qml`), which resolves tokens from the system palette,
the color scheme, the user's overrides and the platform.

### 3.1 Color (semantic)

| Token | Light | Dark | Use |
|---|---|---|---|
| `window` | `#F3F3F5` | `#161618` | Window background, transport bar |
| `sidebar` | `#ECECEF` | `#1D1D20` | Sidebar (solid fallback for Mica/vibrancy) |
| `content` | `#FFFFFF` | `#161618` | Main content surface |
| `raised` | `#FFFFFF` | `#202023` | Cards, panels, artwork placeholder |
| `hover` | `#0000000A` | `#FFFFFF0F` | Hover overlay on Tw components |
| `selected` | accent 12 % (`#E6F1F2` with Tin Teal) | accent 18 % (`#1B2B2D`) | Selected row/tile |
| `separator` | `#0000001A` | `#FFFFFF14` | Hairlines |
| `text` | `#1B1B1F` | `#F2F2F4` | Primary text |
| `textMuted` | `#5D5D66` | `#A6A6AF` | Secondary text |
| `textFaint` | `#6E6E77` | `#8C8C95` | Tertiary text (still ≥ 4.5:1) |
| `accent` | OS accent, fallback Tin Teal `#0B6E78` | OS accent, fallback `#5CC6CF` | Selection, focus, primary action |
| `onAccent` | `#FFFFFF` | `#0F2F33` | Text on accent fills |
| `accentText` | derived | derived | Accent used as text; auto-darkened/lightened until ≥ 4.5:1 |
| `srcServer` | `#1F5FA8` | `#7BB0EE` | Server source badge |
| `srcDownload` | `#2E7D46` | `#76C48E` | Downloaded source badge |
| `warning` | `#9A5B00` | `#E0A453` | Budget > 80 %, opt-in tier |
| `danger` | `#B42318` | `#F2877D` | Wipe, network tier, errors |

- All text pairs are ≥ 4.5:1 on their surface (checked; keep it that way — a token change needs a contrast check).
- Meaning is never carried by hue alone: source, tier and status always have an icon and a word.
- Chrome is never tinted from cover art. Now Playing may sample one muted tone at ≤ 8 % opacity.
- The play/pause button is filled with `text`, not `accent`, so it never fights the artwork.

### 3.2 Type — system font, scaled from the platform base

`base` = the active style's control font size (Windows FluentWinUI3 draws 14 px Segoe UI Variable while
`Qt.application.font` reports 12 px — T-0011; macOS 13, Linux: desktop setting). Token fixes pending in T-0035.

| Token | Size | Weight | Use |
|---|---|---|---|
| `display` | base + 14 | 650–700 | Album/artist page title |
| `title` | base + 6 | 600 | Screen title |
| `subtitle` | base + 2 | 600 | Section heads |
| `body` | base | 400 | Default |
| `bodyStrong` | base | 600 | Track title, now playing |
| `caption` | base − 2 | 400 | Metadata, timestamps |
| `overline` | base − 3, uppercase, +6 % tracking | 600 | Section labels ("NOW PLAYING") |

Durations, counts and sizes use tabular figures (`font.features: { "tnum": 1 }`). No bundled UI font.

### 3.3 Space, size, radius, motion

- **Space (4 px grid):** `xxs 2 · xs 4 · s 8 · m 12 · l 16 · xl 24 · xxl 32 · xxxl 48`.
- **Sizes:** sidebar 232 (rail 56) · transport bar 80 · side panel 320 · row 40 comfortable / 32 compact ·
  album tile min 168 · window min 880 × 560 · hit target ≥ 32 px (mouse), ≥ 44 px for touch-capable layouts.
- **Radius:** `control` Win 4 / mac 6 / Linux 4 · `surface` 8 / 10 / 8 · `artSmall` 4 · `artLarge` 8.
- **Motion:** `fast` 120 ms (hover, press) · `normal` 200 ms (panels) · `slow` 300 ms (view change);
  `Easing.OutCubic` in, `InCubic` out. OS reduced-motion → all durations 0 except progress indicators.
- **Elevation:** none on content. Popups/menus are native and bring their own shadow. Large album art may
  carry one soft shadow.

### 3.4 Icons

One stroke set: **Lucide** (ISC license), shipped as SVG in the Qt resource file, 20 px at 1.75 stroke
(16 px in dense rows), tinted with `text`/`textMuted`. Filled glyphs only for "on" states (favorite, playing).
No emoji, no platform icon fonts. The app icon is the Tinwave mark (wave in a rounded Tin Teal square).

## 4. Layout

```
┌ native title bar ──────────────────────────────────────────────┐
│ Sidebar 232 │ Content (fluid)                     │ Side panel │
│  app menu   │  header: title · count · filters    │ 320, opt.  │
│  search     │  virtualized grid / list            │ Queue ·    │
│  Library    │                                     │ Lyrics ·   │
│  Listen     │                                     │ Info       │
│  Sources    │                                     │            │
│  Manage     │                                     │            │
│  task prog. │                                     │            │
├─────────────┴─────────────────────────────────────┴────────────┤
│ Transport bar 80: now playing + source │ controls + seek │ speed, device, volume, queue │
└────────────────────────────────────────────────────────────────┘
```

- Sidebar groups: **Library** (Songs, Albums, Artists, Genres) · **Listen** (Smart Radio, Insights, playlists)
  · **Sources** (This computer, each server, Downloads) · **Manage** (Import & organize, CD ripping,
  Signal Center, Settings). Spec §14 areas map 1:1.
- Below 1100 px width the side panel overlays; below 960 px the sidebar collapses to a 56 px rail.
- Background tasks (scan, analysis, compaction, sync, downloads) show as `TwTaskProgress` in the sidebar
  footer — never modal, always cancellable.
- Mini-player is a separate small window using the same `TwTransportBar` parts.

## 5. Components

Native controls (Button, ToolButton, TextField, SearchField, ComboBox, Switch, CheckBox, Slider, SpinBox,
TabBar, ScrollBar, Menu, ToolTip, dialogs) are used as-is from the OS style. `Tw*` components live in
`apps/tinwave-desktop/qml/Tw/` and are the only custom-drawn UI.

| Component | Native base | Purpose | Signals it must emit (catalog) |
|---|---|---|---|
| `TwTransportBar` | ToolButtons, Slider | Now playing, controls, seek, speed, device, volume, queue toggle | `PlayStarted`, `Paused`/`Resumed`, `Skipped`, `PreviousPressed`, `RepeatModeChanged`, `ViewOpened` (queue/lyrics) |
| `TwSeekBar` | Slider | Position with section ticks (from analysis); hover shows time + section | `SeekForward`/`SeekBack` on release only |
| `TwVolumeControl` | Slider + ToolButton | App volume, mute | `VolumeChanged` once per gesture (press→release, or wheel burst ended after 600 ms), `MuteToggled` |
| `TwSpeedControl` | ToolButton + Menu | Playback rate | `RateChanged` once per gesture |
| `TwTrackRow` | ItemDelegate | Track in any list; states: default, hover, selected, playing, unavailable | `PlayStarted(origin)`, `QueuedNext/Later`, `Favorited`, `Impression` (counter) |
| `TwAlbumTile` / `TwArtistTile` | ItemDelegate | Grid item with async artwork; hover/focus shows play button | `PlayStarted(origin)`, `ArtworkHover` (counter), `Impression` (counter) |
| `TwArtwork` | Image (async provider) | Bounded-cache thumbnail; flat `raised` placeholder, no shimmer | — |
| `TwSourceBadge` | — | Local / `<server name>` / Downloaded; full or icon-only with tooltip + accessible name | — |
| `TwWhyChip` | ToolButton + Popup | "Why this?" on every recommendation; reasons + feedback actions | `ExplanationOpened(reason_kind)`, `RadioFeedback(kind)` |
| `TwSidebar` / `TwSidebarItem` | ListView, ItemDelegate | Navigation | `PageDwell` measured by the page, not the item |
| `TwTierBadge` | — | T0 Playback · T1 Context · T2 Opt-in · T3 Network | — |
| `TwSignalRow` | Switch | Signal Center category: name, description, count, last event, toggle | toggle writes settings (not a signal) |
| `TwStorageMeter` | ProgressBar | Store size vs ADR 0002 budget; > 80 % adds warning token + word | — |
| `TwTaskProgress` | ProgressBar + Button | Cancellable background task | — |
| `TwEmptyState` | Button | One sentence, one action | — |

States every `Tw*` component must render: default, hover, pressed, keyboard focus (2 px `accent` ring, offset 2),
selected, disabled, and its loading/unavailable state where relevant — in light and dark.

## 6. Writing (UI copy)

- Plain, short, sentence case. "Add folder…" (ellipsis when a dialog follows).
- Explanations speak about the listener's behavior, not the algorithm: "You replay its chorus", not
  "high segment-replay score".
- Privacy copy states exactly what happens: "Stored only on this PC", the literal weather request URL.
- Never blame: "File missing — moved or renamed?" with a Locate… action.

## 7. Accessibility

- Every interactive element is reachable by keyboard, has `Accessible.name` (+ `description` where useful),
  and shows a visible focus ring.
- Contrast: text ≥ 4.5:1, large text and UI glyphs ≥ 3:1, in both themes and with any OS accent
  (use `accentText` for accent-colored text).
- Respect OS reduced motion, high contrast (Windows high-contrast → use system palette only), and text scaling.
- Hover-only affordances always have a keyboard and context-menu equivalent ("Why this?" opens on Enter).

## 8. Definition of Done for UI work (design part)

A UI ticket is not done unless:
- [ ] Uses only `Theme` tokens — no raw colors, font sizes, spacing, radii or durations in QML
      (`qml/` grep for `#[0-9a-fA-F]{3,8}` and bare `font.pixelSize:` numbers outside `Theme/` is empty).
- [ ] Uses native controls for everything in the native list; custom drawing only via `Tw*` components.
- [ ] Renders correctly in light and dark, and with a non-default OS accent.
- [ ] Checked under the native style of at least the dev OS, and the other two styles at least via
      `QT_QUICK_CONTROLS_STYLE` (FluentWinUI3 / macOS / Fusion) where the style is available.
- [ ] Keyboard-only walkthrough works; accessible names present; focus visible.
- [ ] Every interaction emits its catalog signals with input source (emission tests exist).
- [ ] Screenshots (light + dark) attached to the handoff under `reports/T-NNNN/` evidence list.
- [ ] Matches the referenced canvas artboard or this file; deviations listed in the handoff.

## 9. Changing the design system

- New token, new `Tw*` component, or a change to an existing one = a **design ticket** (`type: feature`,
  scope `docs/design/` + `apps/tinwave-desktop/qml/Theme|Tw/`) that updates this file first.
- A new screen or a pattern not covered here → the lead asks the PO for a canvas artboard or a decision
  (user-visible behavior not described in the spec or guides is a PO escalation, work/README.md).
- Changing a principle or the native-first stance → new ADR superseding 0003.
