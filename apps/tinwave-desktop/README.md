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
| `qml/Theme/Theme.qml` | Design tokens (the only place raw colours/sizes may appear) |
| `qml/Main.qml`, `qml/Sidebar.qml` | Window shell |
| `tests/startup.rs` | Headless start-up tests (offscreen platform, software rendering) |
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
- `TINWAVE_MAIN_QML=<url>` loads another root QML file (**debug builds only**; used by the load-failure test).
- Exit codes: `2` the root QML could not be loaded, `3` Qt could not be initialised.

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
