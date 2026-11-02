# tinwave-desktop

Qt 6 Quick/QML window shell over thin CXX-Qt view models. Cargo-driven build, Qt linked dynamically
([ADR 0008](../../docs/adr/0008-cxx-qt-bridge.md)); style and tokens per
[design-system.md](../../docs/design/design-system.md).

## Layout

| Path | What |
|---|---|
| `build.rs` | CXX-Qt QML module `Tinwave.Desktop` (`.crate_include_root(None)`: no whole-crate rebuild watching) |
| `src/main.rs` | Entry point; calls `app::wiring::run()` |
| `src/app/wiring.rs` | Composition root: Qt start-up, style, engine, load-failure exit path |
| `src/app/style.rs` | Style selection (FluentWinUI3 on Windows, Fusion elsewhere; `QT_QUICK_CONTROLS_STYLE` overrides) |
| `src/app/app_info.rs` | First view model (QML singleton `AppInfo`); the only `#[cxx_qt::bridge]` so far |
| `qml/Theme/Theme.qml` | Design tokens (the only place raw colours/sizes may appear) |
| `qml/Main.qml`, `qml/Sidebar.qml` | Window shell |
| `tests/startup.rs` | Headless start-up tests (offscreen platform, software rendering) |

## Build, run, test

```
# Windows (Git Bash); Qt MSVC kit, no CMake needed
export QMAKE='C:\Qt\6.12.0\msvc2022_64\bin\qmake.exe'
export PATH="/c/Qt/6.12.0/msvc2022_64/bin:$PATH"     # Qt DLLs, needed to RUN the app and the test binaries
cargo run -p tinwave-desktop
cargo test -p tinwave-desktop                         # headless: needs no display, opens no audio device
```

Developer switches:

- `QT_QUICK_CONTROLS_STYLE=Basic|Fusion|...` replaces the style chosen at start-up.
- `--color-scheme=light|dark|system` switches the app's own colour scheme in-process (the OS setting is untouched).
- `TINWAVE_MAIN_QML=<url>` loads another root QML file (used by the load-failure test).
- Exit codes: `2` the root QML could not be loaded, `3` Qt could not be initialised.
