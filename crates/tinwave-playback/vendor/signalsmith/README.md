# Vendored Signalsmith Stretch

Header-only C++ libraries by Signalsmith Audio Ltd., MIT licensed, used for pitch-preserving playback rate
(ADR 0006, ticket T-0026). The wrapper that compiles them is `../../src/stretch/` (`tw_stretch.cpp`); `build.rs`
adds this directory to the include path. Nothing here is modified.

| Directory | Project | Pinned version | Upstream |
|---|---|---|---|
| `signalsmith-stretch/` | Signalsmith Stretch | **1.3.1** (`signalsmith-stretch.h`, `version = {1, 3, 1}`), commit `c5987267e6ad42ff9b620b2802c4600472e85818` of 2025-06-14 ("Add `.exact()` method ...") | https://github.com/Signalsmith-Audio/signalsmith-stretch |
| `signalsmith-linear/` | Signalsmith Linear (FFT, STFT) | commit `157b448e390663e78d845ca9f8dad65140f686ad` of 2025-06-12 ("Proxy value for `SplitPointer` ...") | https://github.com/Signalsmith-Audio/linear |

Licences: `signalsmith-stretch/LICENSE.txt` (MIT, Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd.) and
`signalsmith-linear/LICENSE.txt` (MIT, Copyright (c) 2025 Signalsmith Audio). Keep both next to the headers.

## Where the files came from

Copied unchanged from the `signalsmith-stretch` 0.1.3 crate on crates.io (the crate bundles both projects as git
submodules), then checked byte for byte against the upstream commits above (raw files at those commits, SHA-256):

| File | SHA-256 |
|---|---|
| `signalsmith-stretch/signalsmith-stretch.h` | `a1ad98eaaf81380723b5f39ef5389fa5aa27fa62c11c6173701431f96510a786` |
| `signalsmith-stretch/LICENSE.txt` | `ee2ef82481ffb445ecdd4b3a4c1f82c0cddb2da8fe39d8e8dc384fffb3e7f06f` |
| `signalsmith-linear/fft.h` | `fb3409a88cdd4354bafe3286ef38c673f26edebd36bf0d4d002a2acecc1e6037` |
| `signalsmith-linear/linear.h` | `67315815c789026477f74a82a005c498bfc4fcbebe5bad54ca2b0eb8005c463f` |
| `signalsmith-linear/stft.h` | `727e19c35ee1792bf72863f07d96d78a4b3e9669e35374ade5aa523c667988cf` |
| `signalsmith-linear/LICENSE.txt` | `072b5e9eb5b22880bdf6256324654cf3ce53dba828edb911921121e1be09d9c3` |

The crate itself (`signalsmith-stretch`) is **not** a dependency: its build script runs bindgen, which needs
libclang. Only these headers are taken from it.

## What is left out

Upstream's `cmd/`, `web/`, tests, CMake files and `signalsmith-linear/platform/` (Accelerate and IPP back ends).
The back ends are only selected by `SIGNALSMITH_USE_ACCELERATE` / `SIGNALSMITH_USE_IPP`, which nothing defines;
turning either on needs the `platform/` headers copied in first.

## Updating

1. Pick the new upstream commits (or tags), copy the same files, keep the licence files, change the table above.
2. Re-run `cargo nextest run -p tinwave-playback --test stretch`: the latency test pins the numbers the engine
   relies on (ADR 0006 measured them for this version), and the round-trip test measures pitch and length.
3. A new version changes the audio: it needs the same listening check as ADR 0006.

## Licences in the shipped application

`cargo deny` only sees crates, not these headers, and the notices generator
(`apps/tinwave-desktop/packaging/New-ThirdPartyNotices.ps1`) only reads licence files of registry crates, so the two
MIT texts here are not picked up automatically: the packaging notices must list "Signalsmith Stretch" and
"Signalsmith Linear" with these two files. Raised with the lead in the T-0026 handoff.
