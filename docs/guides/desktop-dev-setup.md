# Desktop development setup

Rust–Qt bridge: CXX-Qt 0.10, Cargo-driven, Qt linked dynamically (ADR 0008). Audio: ADR 0004–0006.

## Common

- Rust stable via `rustup` (version pinned in `rust-toolchain.toml`), components `rustfmt`, `clippy`.
- Qt 6.10 or newer (D-0006) with Qt Quick and Qt Quick Controls 2; Qt Multimedia is **not** required. Install via the Qt
  online installer or system packages. Set `QMAKE` to the Qt 6 `qmake` so CXX-Qt finds it.
- CMake and Ninja are not needed for the desktop app: it builds through Cargo (ADR 0008). Keep them only for spike
  prototypes that use CMake.
- Optional: FFmpeg shared libs for fallback decoding/conversion (`FFMPEG_DIR`).
- Tools: `cargo-nextest`, `cargo-deny` (licenses/advisories), `cargo-insta`, `cargo-criterion`.

## Windows

- Visual Studio 2022 or later Build Tools (MSVC, Windows SDK); VS 2026 / MSVC 14.5x works with the
  `msvc2022_64` Qt kits.
- Qt 6 **MSVC 64-bit** kit (`msvc2022_64`) — the MinGW kit cannot link with the `x86_64-pc-windows-msvc`
  Rust toolchain. Install Qt 6.10 or newer (the floor, ADR 0007); the dev machine uses 6.12.0. Add `<Qt>\<version>\msvc2022_64\bin` to
  `PATH` for running. The Qt installer's `Tools\Ninja` works as the Ninja binary.
- WASAPI and WinRT APIs come through the `windows` crate (features enabled per module only).
- The desktop app builds with Cargo only (CXX-Qt, ADR 0008): set `QMAKE=C:\Qt\<version>\msvc2022_64\bin\qmake.exe`;
  no vcvars or CMake needed. Editing a bridge, QML or C++ file costs ~10 s per rebuild (plain Rust ~1 s).

## macOS

- Xcode command-line tools. Qt 6 macOS kit (universal).
- Location / network-place providers require entitlements and `Info.plist` usage strings; dev builds
  must be signed ad-hoc to test permission prompts.

## Linux

- `build-essential`, `pkg-config`, `libdbus-1-dev`, `libpipewire-0.3-dev` (or PulseAudio), `libasound2-dev`.
- Qt 6 dev packages (`qt6-base-dev qt6-declarative-dev`).
- Test on both X11 and Wayland sessions (foreground-app provider is Unknown on Wayland by design).

## Running

```
cargo run -p tinwave-desktop
```

Dev flags (env):
- `TINWAVE_DATA_DIR` — use a throwaway data dir.
- `TINWAVE_FAKE_CONTEXT=fixtures/context/rainy-friday.toml` — inject a fixed context to test radio.
- `TINWAVE_SIGNAL_LOG=1` — mirror signals to stdout in human-readable form (debug builds only).

## Before every PR

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
```
