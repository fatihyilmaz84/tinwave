# ADR 0008 — Rust ↔ Qt bridge: CXX-Qt 0.10, Cargo-driven build, dynamically linked Qt

- **Status:** Accepted (lead, from spike T-0002; report work/reports/T-0002/spike-1.md)
- **Date:** 2026-11-02

## Context

Spec section 2 and Phase 0 require validating Rust + Qt Quick/QML and the bridge before UI work starts
(`docs/rules/qml-ui.md`: thin Rust view models exposed as QObjects, `QAbstractListModel` over paged queries, async artwork
provider, `qt_thread().queue(...)` for results). Spec section 12 sets memory targets (idle 50-100 MB, browsing 80-160 MB,
"must include relevant Qt memory").

The spike (`spikes/T-0002-cxx-qt/`, throwaway) built one app with: a QML `ApplicationWindow` using Qt Quick Controls
**FluentWinUI3**; a `ListView` (`reuseItems`) over a Rust `QAbstractListModel` with **100 000 synthetic rows** served by
paged fetch on a worker thread (200 rows per page, 64 pages resident, LRU eviction, bounded request queue, `data()` never
blocks); an **async image provider** with a bounded Rust LRU cache; a 200 ms Rust task posted back to the Qt thread; and a
`Counters` QObject that debounces forwarded hover/scroll events in Rust. A harness drives it (idle, two flick speeds, a
whole-list traversal, tasks during a flick, call-cost loops) and prints frame times, RSS and CPU.

**Machine:** Windows 11 Pro 10.0.26200, AMD Ryzen 7 7800X3D (8C/16T), ~32 GB RAM, NVIDIA GeForce RTX 4070 (driver
32.0.16.1656) plus AMD iGPU, display at about 144 Hz (frame interval p50 6.9 ms), Windows scaling 150 percent. Qt **6.12.0**
`msvc2022_64`, MSVC 14.50.35717 (VS 2026 Community), Rust 1.92.0, cxx-qt / cxx-qt-lib / cxx-qt-build **0.10.0**, **release**
builds. The desktop was in normal use (browser, other agents building in parallel), so CPU numbers are noisy, see 4.

## Decision

1. **Adopt CXX-Qt 0.10 as the Rust-Qt bridge** for `apps/tinwave-desktop`. It is viable on Windows/MSVC: QObjects, a
   `QAbstractListModel` subclass, properties, signals, invokables, `Initialize`, `Threading` and QML registration all worked
   without hand-written C++, and the generated code passes `clippy -D warnings`.
2. **Cargo-driven build, no CMake.** `build.rs` uses `CxxQtBuilder::new_qml_module(...)`; Rust owns `main`; Qt is found through
   `QMAKE` (and `PATH` for running). CMake (Corrosion + `cxx-qt-cmake`) works too (built and run in the spike) but needs a C++
   `main`, a staticlib crate, `vcvars` for Ninja/MSVC, an unpinned `FetchContent` of two repos, and gives no faster builds.
   Revisit CMake only if a platform packaging step demands it (macOS/Linux packaging is not tested here).
3. **Qt is linked dynamically** (LGPL-3.0 route). Never enable the `link_qt_object_files` feature or a static Qt build.
   Windows deploy with `windeployqt` (flags in 3.5).
4. **Allow a small C++ shim where CXX-Qt cannot subclass a Qt type.** CXX-Qt 0.10 binds `QQmlEngine::addImageProvider` and
   `QQmlImageProviderBase` but cannot subclass `QQuickAsyncImageProvider` / `QQuickImageResponse` (they are not QObject
   bases it can use). The shim is about 85 lines of C++ + a 27-line moc'ed task header, compiled by `cxx-qt-build` through
   `.cpp_file(...)` (headers are moc'ed automatically). Pixels come from a Rust `extern "C"` function that owns the bounded
   LRU. Any further shim needs a note in the ticket that adds it.
5. **Bridge rules learned (to go into `docs/rules/qml-ui.md` / architecture):**
   - `build.rs` must call `.crate_include_root(None)` (or point it at a header dir). The default watches the whole crate
     directory (`rerun-if-changed` on the root, and `.qmlls.ini` is rewritten every build), so **every** `cargo build` re-ran
     the build script: a no-op build cost 10 s. With it: no-op 0.1 s.
   - `main` must connect `QQmlApplicationEngine::objectCreationFailed` (bound as `on_object_creation_failed`) and exit
     non-zero. Without it a QML load error leaves `exec()` running forever with no window (observed: hung until killed).
   - `requestImageResponse()` "may be called by multiple threads" (Qt docs), `finished()` must be emitted even after
     `cancel()`, and responses are `deleteLater`-ed afterwards: report back with a queued signal of a task object
     connected to the response, never by touching the response from another thread (see open risks).
   - `Image.sourceSize` is in logical pixels; Qt multiplies by `devicePixelRatio` before calling the provider (I passed
     48 px and the provider saw 72 px at 150 percent scaling). Do not pre-multiply.
   - `QAbstractListModel::data(&self)` cannot mutate, so LRU stamps / in-flight sets need `Cell`/`RefCell`; request pages
     through a bounded channel to a worker and apply results with `qt_thread().queue(...)`, then emit `dataChanged`.
   - Bridge modules cannot be `#![forbid(unsafe_code)]` (the macro emits `export_name`/unsafe items); `#![deny(unsafe_code)]`
     at the crate root plus `#[allow(unsafe_code)]` on the bridge modules works, and `QQmlEngine::add_image_provider` is an
     `unsafe fn` (ownership transfer) so the composition root needs an allow too. `[workspace.lints] unsafe_code = "forbid"`
     must therefore not be inherited by `apps/tinwave-desktop`.
6. **UI counters (catalog section 5: Impression, ArtworkHover, PageDwell): forward raw events to Rust, debounce in Rust.**
   QML `HoverHandler.onHoveredChanged` and `ListView.onContentYChanged` call `Counters.hover/viewport`, which only send on an
   mpsc channel. A Rust thread blocked in `recv_timeout` applies the 800 ms hover and 500 ms settle rules and costs nothing
   when idle.
7. **Qt floor: recommend PO consider 6.12 instead of 6.8 LTS** (section 6). Not decided here (→ Lead decision 3 below: floor stays 6.10).

## Evidence (all on the machine above, deployed copy run with Qt removed from `PATH`)

### 1. Build

| What | Result |
|---|---|
| Clean build, dev profile (all deps, cxx-qt-lib C++) | 108 s |
| Clean build, release profile | 89 s |
| No-op `cargo build` (after `crate_include_root(None)`) | 0.1-0.2 s |
| Edit plain Rust (`main.rs`, `metrics.rs`) | 1.0-1.3 s (release and dev) |
| Edit a `#[cxx_qt::bridge]` file, a QML file, a C++ file or `build.rs` | **10.2-11.9 s** (build script reruns, recompiles all generated C++ + moc/rcc/qmlcachegen; rustc itself 1.3 s) |
| `cargo check` after a bridge edit / after a plain edit | 10.9 s / 0.4 s |
| CMake variant (Ninja + MSVC + Corrosion): clean / no-op / bridge edit / QML edit / C++ `main.cpp` edit | 88 s / 1.7 s / 11.9 s / 11.9 s / 3.5 s |
| `target/` size after dev + release builds | about 1 GB (dev 0.95 GB) |

- MSVC: Cargo finds `cl.exe` and the SDK itself (no `vcvars` needed for the Cargo route, run from Git Bash). Qt must be the
  **MSVC** kit (the old MinGW kit cannot link with the MSVC Rust target). VS 2026 / MSVC 14.50 is ABI-compatible with the
  `msvc2022_64` kit. CXX-Qt MSRV is Rust 1.85. For the CMake route `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL` is required.
- The 10 s per bridge/QML edit is the main inner-loop cost of CXX-Qt on this machine; keep bridge files few and small, keep
  logic in plain Rust modules (1 s rebuilds), and use `qmlls`/hot reload for QML-only iteration if it matters (not tried).

### 2. Startup, memory, size

Numbers are medians [min..max] over 8 completed full runs (n=8) unless noted. WS = working set (what Process Explorer and
`Get-Process.WorkingSet` show; includes shared DLL pages). PWS = private working set (Task Manager "Memory" column).
Spec section 12 does not say which one the 50-100 MB target means; **the spec should name it** (WS is the conservative one).

| Measurement | Value |
|---|---|
| Release exe | 1.11 MiB (dev 3.60 MiB) |
| `windeployqt` default (QML scan) | 91.9 MiB, 1375 files |
| `windeployqt` trimmed (see 3.5) | **55.4 MiB**, 1084 files (Core 10, Gui 9, Quick 6.4, Qml 5.1, FluentWinUI3 3.4, Basic 2.4, Fusion 2.0, Templates2 1.9, OpenGL 1.9, Network 1.7, QML files 4.1, plugins 2.1) |
| Warm start to first frame (since `main`) | **360 ms** [339..393]; process age 374 ms [354..410]; `engine.load` returns at 193 ms |
| Warm start, wall incl. process spawn (8 runs) | 468-625 ms |
| First execution of a freshly copied folder (n=4, new files: Defender scan) | process age 1.37-1.63 s |
| True disk-cold start | **not measured** (needs cleared standby list / reboot, i.e. admin or system action) |
| Idle (2.5 s after first frame, then 3 s) | **0 frames, 0.2 percent CPU** [0..1.0]; WS 98.2 MB [98.0..99.3], PWS 40.9 MB [40.5..41.3] |
| After flick (200 rows/s, 5 s) | WS 101-103, PWS 44-46 |
| Peak during fast traversal of all 100 000 rows (3 s down, 3 s up, about 33 000 rows/s) | **WS 123.3 MB max, PWS 65.1 MB max**; after settling WS 119-122 and not growing with a second pass |
| Baseline "empty window" same style (n=5 / 2) | WS 84.7 / PWS 29.5 MB at first frame |
| Same, Basic or Fusion style | WS 73.0-73.7 / PWS 19.0-19.5 MB (Fluent costs about +11 MB private) |
| Same, `QT_QUICK_BACKEND=software` (no D3D11 driver) | WS 64.4 / PWS 24.0 MB |

- Idle WS (about 98 MB) already sits at the top of the 50-100 MB target **before** any DB, audio engine, signal pipeline or
  analysis exists; about 65-75 MB of it is the Qt + GPU-driver baseline. PWS (41 MB) is comfortable. Browsing peaks
  (WS 123 MB) are inside the 80-160 MB target.
- Bounded: 911 page fetches and 847 evictions during one full traversal, never more than 64 pages (12 800 rows) resident,
  0 dropped requests; image cache capped at 8 MB (10 000+ requests per run, about 72 percent cache hits).

### 3. Behaviour

1. **Lazy 100 000-row model.** Rows are fetched in pages of 200 off-thread; `data()` returns a placeholder and requests the
   page if missing, so the Qt thread never waits (rows from index 54 321 render correctly, screenshot in `results/`).
2. **Scrolling smoothness at ~144 Hz.** Flick at 200 rows/s: 720 frames in 5 s, frame-interval p99 7.4 ms, max 8-14 ms.
   Whole-list traversal at 33 000 rows/s: 433 frames in 3 s, p99 7.8 ms, max 9-11 ms. Frames over 20 ms: 0 in the large
   majority of phases, one in a few (see noise note).
3. **Rust task to Qt thread without freezing.** A 200 ms CPU task on a Rust thread while the list flicks: frame max **10.5 ms**
   [9.0..49.4] (3 tasks per run, 7 valid runs) versus the **control** (same 200 ms run on the Qt thread): frame max **200.7 ms**
   [200.5..201.3]. So the detector works and posting does not freeze the UI. Post latency (worker done to closure running on
   the Qt thread, n=24): median **88 microseconds**, max 483 microseconds.
4. **Async image provider.** One request per cover (32 logical px, 48 px at 150 percent), 2 pool threads, newest request
   first, cancellation flag, Rust LRU. Works in the deployed build. The 3 ms "decode" is a sleep (synthetic).
5. **Counters.** QML to Rust invokable call: hover **248 ns**, viewport **245 ns** (200 000 calls each, release), versus
   733 ns for the QML-side `indexAt()` used to compute the visible range. Two calls per frame at 144 Hz is far below 0.1 percent
   of a core. Frame times with forwarding on vs off are indistinguishable (p99 7.4-7.9 ms vs 7.4-8.9 ms). Debounce
   self-test: a 300 ms hover is not counted, a 1.1 s hover counts exactly once; viewport changes produce one "settled" batch
   after 500 ms of stillness. In an early run the real mouse pointer resting over the window produced 338 genuine
   `HoverHandler` events that arrived in Rust (not synthetic).
6. **Native styling.** FluentWinUI3 renders correctly with OS dark mode and 150 percent scaling (`results/screenshot-fluent-os-dark.png`);
   picked up from the OS with no code. See open questions for what Qt's docs say about "native".

### 3.5 Deploy recipe (self-contained, verified with Qt off `PATH`)

```
windeployqt --release --qmldir qml --no-translations --no-opengl-sw --no-system-d3d-compiler --no-system-dxc-compiler \
  --skip-plugin-types qmltooling,generic --no-quickcontrols2material --no-quickcontrols2universal \
  --no-quickcontrols2imagine --no-quickcontrols2windows --no-quickcontrols2native <dir>\app.exe
# then delete qml\QtQuick\Controls\{Material,Universal,Imagine,Windows,Native} and the matching *StyleImpl.dll
```
**FluentWinUI3 imports `QtQuick.Controls.Fusion`** (its `qmldir`), so Fusion must stay in the bundle: removing it made the
QML load fail. Qt's docs list Dial, Drawer, header views, SplitView, StackView, SwipeDelegate, SwipeView, TreeViewDelegate and
Tumbler as falling back to Fusion in this style.

### 4. Noise

CPU% per phase (process CPU time / wall) is **bimodal and not attributable**: the same 5 s flick gave 0.8 percent in some runs
and 37-50 percent in others, independent of counter forwarding (on/off medians differ in both directions). It is probably
GPU-driver/present-thread behavior on this two-GPU machine, not our code. Treat it as "1-50 percent of one core while
scrolling at 144 Hz" and re-measure on other hardware. Frame-time rows with an occluded window (no frames presented) were
excluded (1 of 8 runs, two phases). Another agent was building on the same machine for part of the session.

## Options compared

| Option | Verdict | Why |
|---|---|---|
| CXX-Qt 0.10, Cargo-driven | **Chosen** | Everything in the spike except the image provider needed no C++; one toolchain; clippy-clean generated code; Rust-owned `main`. |
| CXX-Qt via CMake (Corrosion + cxx-qt-cmake) | Works, not chosen | Same compile cost, needs C++ `main`, vcvars, runtime-library override, `FetchContent` of unpinned `main` branches. Keep as fallback for packaging. |
| `cxx` + hand-written C++ QObjects | Not chosen (estimate, not built) | Needs a C++ class + header per view model (Q_OBJECT, properties, signals, `Q_INVOKABLE`), our own moc step in `build.rs` (or CMake AUTOMOC + Corrosion), and hand-written thread-safe posting from Rust threads to a QObject pointer (what `qt_thread().queue` already is). Rough size for this spike's `TrackModel` alone: C++ header + cpp about 130 lines + cxx bridge about 35 + glue vs. a 77-line bridge declaration block in CXX-Qt (whole `model.rs` is 328 lines including all logic that would exist in Rust either way). Two languages and two build steps per view model; 12 view models are planned. It stays the escape hatch: the image-provider shim proved it can coexist (`cpp_file` + `moc`). |
| Qt Widgets / QML-only without Rust objects | Out of scope | Not a bridge. |

## Consequences

- Inner loop: bridge/QML/C++ edits cost about 10 s each; keep logic out of bridge files (1 s rebuilds).
- `apps/tinwave-desktop` needs its own lint block (`deny` + targeted `allow` for bridge modules and the composition root).
- Windows packaging uses `windeployqt` with the flags above; ship about 55 MB of Qt. macOS/Linux deploy not tested.
- Memory: idle WS is about 98 MB for an empty list app; spec section 12 should say whether targets are WS or PWS, and the
  50-100 MB idle target has almost no headroom if it means WS on Windows.
- A bridge crate cannot use `forbid(unsafe_code)`; reviews must check `unsafe` only in the allowed modules.
- Qt LGPL obligations (section "Licenses") apply to every release.

## Licenses

| Component | License | Concern |
|---|---|---|
| CXX-Qt (`cxx-qt`, `-build`, `-lib`, `-gen`, `qt-build-utils`) 0.10.0 | MIT OR Apache-2.0 | none |
| `cxx`, `cxx-gen`, `cxxbridge-*`, `link-cplusplus` | MIT OR Apache-2.0 | none |
| Other crates in the spike (57 total) | MIT OR Apache-2.0, MIT, Apache-2.0, Zlib, Unlicense OR MIT, `(MIT OR Apache-2.0) AND Unicode-3.0`, `r-efi` MIT OR Apache-2.0 OR LGPL-2.1-or-later | all within `deny.toml` allow list (pick MIT where dual); `windows-sys` is spike-only |
| Qt 6.12.0 modules shipped: Core, Gui, Network, OpenGL, Qml, QmlMeta, QmlModels, QmlWorkerScript, Quick, QuickControls2 (+Basic, FluentWinUI3, Fusion, Impl, style impls), QuickTemplates2, QuickLayouts, QuickShapes, QuickEffects, Svg, plugins qwindows / qjpeg / qgif / qico / qsvg / qsvgicon / schannel+certonly TLS / networklistmanager | per the kit's SPDX SBOMs: `LicenseRef-Qt-Commercial OR LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only` | **LGPL-3.0 obligations**, see below |
| Qt third-party code inside those modules (from the SBOMs) | FreeType FTL OR GPL-2.0, HarfBuzz and others MIT, BSD-2/3, Apache-2.0, Zlib, MPL-2.0 (Qt Network public-suffix list data), ICC profile license | list goes in the notices file |
| Not shipped, GPL-only: `QuickControlsTestUtilsPrivate`, `QuickTestUtilsPrivate` (commercial OR GPL-3.0 only) | test-only private modules | never link or ship them |

LGPL-3.0 implications for Tinwave: dynamic linking only (Qt DLLs stay separate files that a user can replace); no static Qt,
no `link_qt_object_files`; ship the LGPL-3.0 and GPL-3.0 texts and a Qt attribution/notice file; offer the corresponding Qt
source (the exact version we ship); do not add terms that forbid replacing Qt or reverse engineering for debugging. Tinwave's
own MIT/Apache code can stay permissive. We never modify Qt. `cargo deny` does not see Qt, so a THIRD-PARTY-NOTICES step is
needed (follow-up).

## 6. Qt version floor (the ticket asked what depends on more than 6.8)

Measured on 6.12.0 only; no 6.8 kit is installed. What I found from Qt docs:

- Nothing in the spike code needs more than Qt 6.8: `QQuickAsyncImageProvider`, `QRunnable::create`, `reuseItems`,
  `HoverHandler`, `required property`, `Qt.styleHints.colorScheme` (6.5+), `objectCreationFailed` (6.4+). CXX-Qt itself
  supports Qt 5.15 and all of Qt 6.
- The **FluentWinUI3 style exists only since Qt 6.8** ("Since: Qt 6.8") and Qt says it is still "under development, some
  controls are not yet supported". Behaviour measured here (11 MB more private memory than Basic, no idle redraws, the Fusion
  dependency) is on 6.12 and **may differ on 6.8.x**. A first attempt with an always-created (not running)
  `BusyIndicator` kept rendering at the display rate for about 5.5 s after startup (about 420 frames) before going quiet, so I
  made it visible only while running; the first run with it created on demand showed one 161 ms frame gap (cause inferred,
  not isolated). These are style behaviours to re-check on 6.8 and in T-0011.
- Qt's own docs call FluentWinUI3 "**not a native style**, can run on all supported platforms". ADR 0003 and design-system
  section 2 describe it as the OS-drawn native style. That wording needs a look in T-0011 (the separate `Windows` / `Native`
  styles exist in the kit).
- Support policy (Qt Releases page): 6.8 LTS is supported to 2029-10-08 but **only its initial patch releases are available
  to open-source users**; later 6.8.x go to commercial customers first. The same page lists Qt 6.12 with standard support to
  2031-09-30; "every fourth minor gets LTS releases" suggests 6.12 is the next LTS (the table cell says "No", so confirm).
  Recommendation for PO: make **6.12 the floor** (it is also the only kit on the dev machine) and re-run the harness on a
  6.8.x kit only if 6.8 compatibility is still wanted. A floor change is an ADR + PO decision.

## Open risks

- **Intermittent access violation in `Qt6Core.dll`** (same RVA 0xe9f73, between `QObject::qt_static_metacall` and the next
  export, i.e. QObject/signal machinery) seen **5 times** in the Windows Event Log during the first 1.5 hours of runs of the deployed
  build. Not reproduced afterwards: 0 crashes in 8 + 8 (4 in parallel) + 3 + 12 empty runs on the old code and 13 full
  runs on the final code. A crash handler (`metrics.rs`) never captured a stack. The first image-provider shim delivered the
  result by calling into the response from the GUI thread through a `QPointer` (a cross-thread access to an object owned by
  Qt's image loader, which the docs pattern avoids); it was replaced by the queued-signal pattern. I could not prove that
  was the cause. Needs a debug-symbol run (Qt PDBs / ASan) before the
  provider is reused; treat async provider + QObject lifetimes as the highest-risk code in the bridge.
- Idle memory has no headroom against the 50-100 MB target if it is measured as WS (see 2).
- Scrolling CPU in the tens of percent on some runs is unexplained (4).
- Not measured: true cold disk start, other GPUs, integrated-GPU-only machines, macOS/Linux, Qt 6.8.
- `add_image_provider` and the shim rely on ownership transfer to the engine; wrong order at shutdown is easy to get wrong.

## Spec / docs updated (proposed, not done by the spike)

- `tinwave-specs.md` section 2/16: CXX-Qt chosen; Qt floor outcome; section 12: name the memory metric (WS vs private WS).
- `docs/guides/desktop-dev-setup.md`: Windows route (QMAKE + Cargo; CMake optional), `windeployqt` flags, `crate_include_root(None)`.
- `docs/rules/qml-ui.md`: image-provider threading rule, `objectCreationFailed`, logical `sourceSize`, counters forwarding recipe.
- `docs/rules/rust.md` / workspace `Cargo.toml`: lint override for the desktop crate.
- `docs/design/design-system.md` and ADR 0003: FluentWinUI3 is not an OS-native style (T-0011), Fusion dependency, Qt floor.

## How to reproduce

```
# Git Bash, from spikes/T-0002-cxx-qt
. ./env.sh                       # QMAKE + PATH for Qt 6.12.0 msvc2022_64, Ninja, CMake
cargo build --release            # needs MSVC build tools; no vcvars required
powershell -File run.ps1         # full harness against target\release (Qt on PATH)
# deploy: see 3.5, then:
powershell -File run-deployed.ps1 [-Forward 0] [-Mode empty] [-Style Basic]   # Qt removed from PATH
powershell -File startup.ps1 -N 8 [-Fresh]                                    # startup to first frame
python -I summarize.py run-deployed                                           # medians over results/run-deployed-*.txt
# CMake variant: cmake-variant\prepare.sh, then cmake-variant\cmake-build.cmd configure | build
```
Raw outputs are in `results/` (`summary-forward-on.txt`, `summary-forward-off.txt`, startup files, screenshot).


## Lead decisions (2026-11-02)

1. CXX-Qt 0.10, Cargo-driven (no CMake); CMake/Corrosion stays a packaging fallback; cxx + hand-written QObjects is
   the escape hatch.
2. Qt is linked dynamically only (LGPL-3): never static Qt, never the `link_qt_object_files` feature; ship LGPL/GPL
   texts, a THIRD-PARTY-NOTICES file and a Qt source offer; never ship the GPL-only test-utils modules (T-0042).
3. Qt floor stays **6.10** (PO decision D-0006). The spike's 6.12 suggestion is noted: nothing measured needs more than
   6.8/6.10, and Windows/macOS bundle Qt; revisit only with a concrete need.
4. Lint: `apps/tinwave-desktop` uses crate-level `deny(unsafe_code)` with `allow` only on `#[cxx_qt::bridge]` modules
   and the documented FFI shims (same pattern as tinwave-context/-playback), plus `undocumented_unsafe_blocks = "deny"`.
   CXX-Qt 0.10 rejects attributes other than `doc` and `cxx_qt::bridge` on the bridge module itself, so the
   `#[allow(unsafe_code)]` goes on the `mod name;` declaration in the parent module (T-0040).
5. Async image provider: use the T-0041 pattern below (queued hand-off + explicit quiesce before the engine is
   dropped). The original RVA 0xe9f73 crash was not reproduced; a teardown crash of the same class was, and is fixed.
6. Memory is judged by private working set (D-0006): idle 41 MB PWS for the prototype.

## Addendum: what the first app crate taught (T-0040, 2026-11-02)

- `apps/tinwave-desktop` is a library plus a thin binary: a bin-only crate failed to link its integration tests
  (LNK2019).
- Every binary that links the crate — the app and every `cargo test` binary — needs the Qt `bin` folder on `PATH`;
  without it Windows fails with STATUS_DLL_NOT_FOUND (0xc0000135).
- The headless startup tests (offscreen platform, no display, no audio) run in the default `cargo test`; the workspace
  needs Qt to build anyway.
- Qt 6.12 writes log output to a piped stderr only with `QT_FORCE_STDERR_LOGGING=1`; tests that read Qt warnings set it.
- Measured on the shell: window visible 312 ms median warm, idle private working set 39 MB (FluentWinUI3).

## Addendum: image provider lifetime (T-0041, 2026-11-02)

Report: work/reports/T-0041/spike-1.md; harness and results: spikes/T-0002-cxx-qt/crash-T-0041/.

- Qt's pixmap reader thread calls `requestImageResponse()`, so every response is a QObject owned by that thread;
  `finished()` is delivered to the reader, which calls `deleteLater()` through `sender()`.
- **Crash A (reproduced, stack captured):** emitting `finished()` twice (e.g. from `cancel()` and again on delivery)
  crashes in `QObject::deleteLater` from `QQuickPixmapReader::asyncResponseFinished` — 120/120 Release runs.
- **Crash B (reproduced in the real Rust app, 4/400):** the queued-signal pattern alone crashes in the reader-QThread
  teardown when pool tasks are still in flight while the engine is destroyed. Fixed by an explicit quiesce:
  0 failures in 2040 runs (1640 harness incl. Debug/ASan/Release + 400 real app). Without the explicit call (destructor
  only) 2/300 runs hung at exit.
- 0xe9f73 is a read through freed QObject connection data in the signal-emission tail, seen only in runs ending with
  `Qt.quit()` — most likely crash B's class, not proven.

**Required pattern (also in docs/rules/qml-ui.md):**
1. A response is created, signalled, cancelled and deleted only on Qt's reader thread; workers never hold a pointer to
   it (not even a `QPointer`).
2. The task is a QObject + QRunnable (autoDelete); its only channel to the response is a queued `done(QImage)` signal
   connected with the response as context; `emit done()` is the last statement of `run()`.
3. Exactly one `finished()` per response; `cancel()` only sets an atomic flag; a cancelled task still emits `done()`
   with an empty image.
4. A bounded `QThreadPool` in shared state held by provider, responses and tasks, newest-request-first, with a closing
   flag checked before work. *(T-0044)* The check and `pool.start()` are atomic with respect to setting the flag: one
   mutex, held by `quiesce()` while it sets `closing` (not across `waitForDone()`) and by the response constructor
   around "check `closing` + `pool.start()`". Otherwise a task can start after `waitForDone()` returned.
5. Teardown: set closing + `waitForDone()` on the GUI thread after `exec()` returns and before the engine is dropped.
   In the app this is enforced by an RAII owner (engine + providers, Drop runs every provider shutdown first) — lead
   decision; the provider destructor repeats it. *(T-0044)* The Rust-side handle must outlive every task: quiesce,
   then drop the handle. If a task were ever the last holder of the shared state, its destructor (and the pool's)
   would run on a pool thread and wait on that same pool. No exit path may skip the quiesce: no `process::exit` —
   a QML load failure sets a flag, calls `engine.exit(2)` and returns through normal teardown.
6. The stress target (crash-T-0041/run-batch.ps1: random exit 200–900 ms, 1 pool thread, ~300 Release runs) is re-run
   on any change to a provider. In the app: `apps/tinwave-desktop/tools/stress/run-stress.ps1` (dev-only
   `image-stress` binary; never packaged).

Bridge build notes (T-0044):
- cxx-qt-build 0.10 needs every `#[cxx_qt::bridge]` file of one QML module in the same directory ("Only one directory
  is supported per QmlModule"); keep logic elsewhere and the bridge files together (`src/app/`).
- cxx has no `c_void`: pass an opaque Rust context as `*const u8`. The bridge macro drops `# Safety` docs on bridged
  `unsafe fn`s, so `clippy::missing_safety_doc` is allowed on the `mod` declaration (with
  `missing_debug_implementations` for opaque C++ types) and the safety contract is written next to the declaration.
