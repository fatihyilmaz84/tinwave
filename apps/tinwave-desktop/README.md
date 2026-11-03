# tinwave-desktop

Qt 6 Quick/QML window shell over thin CXX-Qt view models. Cargo-driven build, Qt linked dynamically
([ADR 0008](../../docs/adr/0008-cxx-qt-bridge.md)); style and tokens per
[design-system.md](../../docs/design/design-system.md).

## Layout

| Path | What |
|---|---|
| `build.rs` | CXX-Qt QML module `Tinwave.Desktop` (`.crate_include_root(None)`: no whole-crate rebuild watching) |
| `src/main.rs` | Entry point; calls `app::wiring::run()` |
| `src/app/wiring.rs` | Composition root: `Shell` (Qt start-up, style, engine, artwork provider, load-failure exit path) and `run()` |
| `src/app/ui_owner.rs` | `UiOwner`: owns the QML engine and every provider; quiesces all providers before the engine is dropped (ADR 0008) |
| `src/app/artwork_bridge.rs` | Unsafe boundary of the artwork provider: cxx declarations of the C++ shim, `extern "C"` callbacks, safe `ArtworkProvider` handle |
| `src/image/` | Artwork logic in safe Rust: injected `ArtworkSource`, bounded byte-budgeted LRU cache, per-provider core and counters |
| `cpp/` | C++ shim: `QQuickAsyncImageProvider` + bounded `QThreadPool` + moc'ed task (lifetime rules at the top of `image_provider.cpp`) |
| `tools/stress/` | Dev-only teardown stress target (`image-stress` binary, `Storm.qml`, `Settle.qml`, `run-stress.ps1`) |
| `src/app/style.rs` | Style selection (FluentWinUI3 on Windows, `macOS` on macOS, Fusion elsewhere; `QT_QUICK_CONTROLS_STYLE` overrides) |
| `src/app/app_info.rs` | First view model (QML singleton `AppInfo`); bridge files must all sit in `src/app/` (cxx-qt-build, QTBUG-93443) |
| `src/app/platform_bridge.rs` | QML singleton `PlatformSettings`: mirrors the platform controller (reduced motion, high contrast, transparency, backdrop, native menus); `Main.qml` binds it into `Theme` |
| `src/platform/` | OS services behind traits (settings reader and watcher, `WindowEffects`, `MenuStyle`); `controller.rs` holds every decision with no Qt or OS dependency, `windows.rs` is the only module of the layer with `unsafe`, `unsupported.rs` answers `Unsupported` everywhere else |
| `qml/Theme/Theme.qml` | Design tokens (the only place raw colours/sizes may appear) |
| `qml/Main.qml`, `qml/Sidebar.qml` | Window shell |
| `qml/Tw/` | `Tw*` components (`TwIcon`) and the `TwIcons` URL helper (singleton) |
| `icons/` | Vendored Lucide SVGs (only the used ones; `check_icon_usage` in build.rs enforces it), `VERSION`, ISC licence |
| `tools/vendor-icons.ps1` | Dev-only: fetch and pin Lucide SVGs at stroke 1.75 (network, run by hand only) |
| `tools/capture.ps1` | Dev-only: UI screenshots (`-Mode window` cropped to the DWM frame, or `-Mode desktop` for Mica and native menus); keyboard, `@click`, `@down`/`@up` and `@wait` steps with a readiness check; restores the cursor; see the script header |
| `packaging/` | Windows package: `package.ps1` (build + windeployqt + legal files + checks), `test-packaging.ps1`, generated `THIRD-PARTY-NOTICES.txt` |
| `tests/startup.rs` | Headless start-up tests (offscreen platform, software rendering) |
| `tests/platform_qml.rs` | Headless tests of the platform bridge as QML sees it (`tests/qml/PlatformProbe.qml`: the real shell, `Theme` bindings, developer switches); debug builds only |
| `tests/image_provider.rs` | Headless provider tests: exactly one `finished()` per response, cache, cancel storm + quit with requests in flight |

## Build, run, test

```
# Windows (Git Bash); Qt MSVC kit, no CMake needed
export QMAKE='C:\Qt\6.12.0\msvc2022_64\bin\qmake.exe'
export PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH"     # Qt DLLs, needed to RUN the app and the test binaries
cargo run -p tinwave-desktop
cargo test -p tinwave-desktop                         # headless: needs no display, opens no audio device
```

Developer switches:

- `QT_QUICK_CONTROLS_STYLE=Basic|Fusion|...` replaces the style chosen at start-up. This is Qt's own documented switch, honoured in release builds too (design-system §8).
- `--color-scheme=light|dark|system` switches the app's own colour scheme in-process (the OS setting is untouched).
- `--accent=#rrggbb` overrides the app's accent colour in-process (pale or dark accents for the UI Definition of Done; the OS accent is untouched). Anything but `#rrggbb` is refused with a warning.
- `--transparency=off|system` (`off` forces the solid sidebar instead of Mica, to screenshot the fallback; the OS "Transparency effects" setting is untouched).
- `TINWAVE_MAIN_QML=<url>` loads another root QML file (**debug builds only**; used by the load-failure test).
- Exit codes: `2` the root QML could not be loaded, `3` Qt could not be initialised.

## Platform bridge (Windows)

`src/platform/` reads and follows OS facts Qt 6.10 does not give us, behind traits so the decisions are unit-tested with fakes:

- **Reduced motion, high contrast, transparency**: `SystemParametersInfo` (`SPI_GETCLIENTAREAANIMATION`, `SPI_GETHIGHCONTRAST`) and the `EnableTransparency` registry value. The main window's procedure is subclassed to re-read them when Windows sends `WM_SETTINGCHANGE` (also `WM_THEMECHANGED`, `WM_SYSCOLORCHANGE`, `WM_DWMCOLORIZATIONCOLORCHANGED`). `PlatformSettings` exposes them and `Main.qml` binds the first two into `Theme` (`reducedMotion` turns every duration to 0).
- **Mica** behind the sidebar (Windows 11 22H2+, build 22621): `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)` plus `DwmExtendFrameIntoClientArea`. It needs `QT_QPA_DISABLE_REDIRECTION_SURFACE=1` and an alpha buffer before the window exists; `app::wiring::run()` sets both through `prepare_process()` (the variable goes through the C runtime's `_putenv_s`, because Qt does not see `std::env::set_var` on Windows: a white sidebar otherwise). The sidebar stays solid (`Theme.sidebar`) when the OS build is older, "Transparency effects" is off, a high-contrast theme is active, or `--transparency=off` is given; `PlatformSettings.backdropStatus` says which.
- **Dark native menus**: the sidebar's app menu is `Popup.Native` when `PlatformSettings.nativeMenus` is true, `Popup.Window` otherwise. The opt-in is the undocumented `uxtheme.dll` ordinal 135 (`SetPreferredAppMode`) and 136 (`FlushMenuThemes`), looked up at run time on builds >= 18362; a missing symbol or an older build selects the Qt-drawn window menu automatically (ADR 0007, D-0006). The menu follows the OS scheme, or the app's own `--color-scheme` override.

Nothing here changes a Windows setting. Live OS toggles (accent, light/dark, animation effects, contrast themes, transparency) are verified by hand with `spikes/T-0011-native-styles/PO-RUNBOOK.md`; the unit tests deliver synthetic messages to a message-only window of their own.

Packaging (Windows): `pwsh packaging\package.ps1` with the Qt env set produces a self-contained folder under
`target\package\` that runs with Qt off PATH, then checks it (licences, no GPL-only or static Qt, only
`tinwave-desktop.exe`). It needs the Qt kit's Documentation, Licenses and SBOM components and the Visual Studio C++
redistributable DLLs; see `packaging/README.md`. Run `packaging\test-packaging.ps1` after any change to the recipe.

## Artwork provider and the teardown stress target

QML loads thumbnails with `Image { source: "image://art/<key>"; sourceSize: Qt.size(w, h); asynchronous: true; cache: false }`
(`sourceSize` in logical pixels; Qt multiplies by the device pixel ratio; `cache: false` so the Rust LRU is the only cache).
`main` wires `NoArtwork` (no key has artwork) until the library provides an `ArtworkSource`.

Any change to the provider, the C++ shim or `UiOwner` re-runs the stress target (ADR 0008, testing.md):

```
cargo build --release -p tinwave-desktop --bin image-stress
powershell -File apps/tinwave-desktop/tools/stress/run-stress.ps1 -Count 300      # Release, 1 pool thread, random quit 200-900 ms
powershell -File apps/tinwave-desktop/tools/stress/run-stress.ps1 -Count 200 -Mode scroll -Pool 2 -Parallel 4
```

It must report `crashes=0 hangs=0 bad_output=0`.
