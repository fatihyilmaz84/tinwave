# Windows packaging

Scripted, repeatable packaging of `tinwave-desktop.exe` into one self-contained folder, with the legal files that Qt's LGPL-3.0
and the other third-party licenses require ([ADR 0008](../../../docs/adr/0008-cxx-qt-bridge.md) decision 3, section 3.5 and
"Licenses"). PowerShell only (Windows PowerShell 5.1 or 7); nothing is installed or changed outside the process.

```
$env:QMAKE = 'C:\Qt\6.12.0\msvc2022_64\bin\qmake.exe'      # the documented build environment; Qt need not be on PATH
powershell -File apps\tinwave-desktop\packaging\package.ps1  # -> target\package\tinwave-desktop-windows-x64\
```

Options: `-OutDir`, `-QtDir` (instead of `QMAKE`), `-CrtDir` (MSVC redistributable folder), `-SkipBuild` (use the existing
`target\release\tinwave-desktop.exe`), `-NoSmoke` (headless machines: no window is opened).

## What `package.ps1` does

1. `cargo build --release --locked -p tinwave-desktop --bin tinwave-desktop`. Only that binary: `--workspace` also builds the
   dev-only `image-stress.exe` (T-0044 review), and the script copies **one file** from `target\release`, never `*.exe`.
2. `windeployqt` with the ADR 0008 flags (no translations, no software OpenGL, no system D3D/DXC compilers, no qmltooling/generic
   plugins, no Material/Universal/Imagine/Windows/Native styles), then deletes the style folders and `*StyleImpl.dll` files that
   the QML scan still copied. **Fusion stays**: FluentWinUI3 imports `QtQuick.Controls.Fusion`.
3. Copies the MSVC runtime DLLs that the bundle imports (`vcruntime140*.dll`, `msvcp140*.dll`, ...) from the Visual Studio
   redistributable folder, app-local. The folder is found through `VCToolsRedistDir`, then `vswhere`, or `-CrtDir`. The script
   fails if the runtime is imported and cannot be found (no silent reliance on an installed redistributable).
   windeployqt's own `--compiler-runtime` copies the 18 MB `vc_redist` installer instead, so it is switched off.
4. Legal files (below).
5. `check-bundle.ps1`, then 6. `smoke-test.ps1`. Either one failing makes the script exit 1.

It prints the bundle size and file count at the end. Reference run (Qt 6.12.0, MSVC 14.50 runtime): **1094 files, 57.7 MiB**
(51.4 MiB in the root: Qt DLLs, exe, runtime; `qml\` 4.1 MiB in 1052 files; plugins 2.2 MiB; legal files 0.3 MiB).

## The checks (`check-bundle.ps1`, exit code 1 on any FAIL)

| Check | Fails when |
|---|---|
| `shared-qt` | the kit's `mkspecs\qconfig.pri` is not a shared build, or a crate in the build enables `link_qt_object_files` (`cargo metadata` features) |
| `static-qt` | `tinwave-desktop.exe` does not import `Qt6Core.dll` plus `Qt6Gui.dll`/`Qt6Qml.dll` (PE import table), i.e. Qt is inside the exe |
| `gpl-only` | a bundled DLL is a Qt test module or has no SBOM entry, or the kit's SBOM says its license has no LGPL/permissive option; the sources (`build.rs`, `Cargo.toml`, Rust, C++, QML) or the exe mention a GPL-only module (`QuickTestUtils`, `QuickControlsTestUtils`, any `TestUtils`). In this kit those two modules are **static libraries**, so linking them would put GPL-only code into the exe: hence the source and string checks, not only the DLL list |
| `only-app-exe` | the bundle holds any `.exe` other than `tinwave-desktop.exe`, or `.pdb/.lib/.exp/...` files |
| `closure` | any DLL/exe imports a DLL that is neither in the bundle root nor a Windows system DLL; the MSVC runtime must be in the root |
| `legal` | `THIRD-PARTY-NOTICES.txt`, `SOURCE-OFFER-Qt.txt` or `licenses\Qt-LGPL-3.0-and-GPL-3.0.txt` is missing, or the last one lacks the LGPL-3.0 or GPL-3.0 text |

`smoke-test.ps1` starts the bundle with `PATH` reduced to Windows and the `QT_*`/`QML_*` variables cleared, waits for a visible
window titled `Tinwave`, checks that every loaded Qt DLL (and `qwindows.dll`, the FluentWinUI3 plugin) comes from the bundle
folder, then kills the process. It fails on early exit, on no window within 30 s, and on Qt errors in stderr.

`test-packaging.ps1` proves the checks have teeth: it breaks a copy of a good bundle in 12 ways (GPL-only DLL, GPL-only string in
the exe, GPL-only module in `build.rs`, no dynamic Qt, `image-stress.exe`, missing runtime, missing `Qt6Core.dll`, missing source
offer, smoke test with a missing DLL while Qt is on the caller's PATH, ...) and asserts exit code 1 plus the FAIL line. Run it
after any change to these scripts.

## Legal files in the bundle

| File | Source |
|---|---|
| `THIRD-PARTY-NOTICES.txt` | `New-ThirdPartyNotices.ps1`: Tinwave's license, the Qt LGPL-3.0 statement, the Qt modules and plugins actually in the folder, third-party code inside Qt with full license texts, the MSVC runtime, the Rust crates and their license texts, and every `LICENSE*` under `apps\tinwave-desktop\icons\` when that folder exists (the Lucide ISC text from T-0043) |
| `licenses\Qt-LGPL-3.0-and-GPL-3.0.txt` | `<Qt>\Licenses\LICENSE`, Qt's own file holding both texts |
| `licenses\Apache-2.0.txt` | `license-texts\Apache-2.0.txt` (Tinwave is `MIT OR Apache-2.0`) |
| `SOURCE-OFFER-Qt.txt` | `templates\SOURCE-OFFER-Qt.txt` with the kit's version filled in |

`packaging\THIRD-PARTY-NOTICES.txt` is a committed copy of the generated file so a reviewer can read it; `package.ps1` warns when
the generated one differs. Refresh it with `New-ThirdPartyNotices.ps1 -Bundle <bundle>` (default output is that file) and commit.

### Where the notice data comes from (all installed, nothing downloaded)

- **Rust crates**: `cargo metadata --locked --filter-platform x86_64-pc-windows-msvc`; the normal-dependency closure of the
  package, minus proc-macro crates (they run in the compiler). License texts are the files inside each crate's registry
  source. A crate that ships none (`cxx-qt`, `cxx-qt-lib`) gets the canonical text of each SPDX id of its expression from
  `license-texts\` (MIT with the `Cargo.toml` authors or repository as holder); a missing canonical text fails the run. MPL-2.0 crates
  (Symphonia, D-0003) get a source/unmodified statement.
- **Qt**: the SPDX SBOMs in `<kit>\sbom` decide which modules and plugins are under which license and which third-party
  components belong to which module (only modules whose DLLs are in the bundle count). The text of each component is Qt's own
  attribution page from `<Qt>\Docs\Qt-<ver>\*\*-attribution-*.html`, which is `qtattributionsscanner`'s qdoc output with the
  license text embedded. `qtattributionsscanner.exe` itself is in the kit but scans `qt_attribution.json` files that exist only
  in Qt *source* trees (a binary kit has none for qtbase/qtdeclarative), so it cannot be run here. Components Qt publishes no
  page for come from `qt-supplements\` (verbatim from the Qt source tag; today only XSVG).
- The bundled libraries inside Qt (zlib, PCRE2, libpng, libjpeg, FreeType, HarfBuzz) are mapped to Qt modules by a table in
  the generator, guarded by a check of `config_qtbase.summary` ("Using system X ... no").

The generator **fails** instead of skipping: an SBOM component with no page and no supplement, a crate with no license text,
a Qt DLL with no SBOM entry. Needs the Qt kit's `Documentation`, `Licenses` and SBOM files (all in the standard installer).

## Upgrading Qt

Re-run `package.ps1` and `test-packaging.ps1`. New components appear as generator errors: add the missing `LICENSE*` file to
`qt-supplements\` (from the new Qt tag) or extend the bundled-library table. Update `SOURCE-OFFER-Qt.txt` only through the
template; the version is substituted. Check the trimmed bundle still starts (the smoke test does).

## Not covered

macOS and Linux packaging, an installer, code signing, update delivery. The source offer names Qt's archive as the place for the
exact Qt version; a Tinwave-side contact for the offer is a project decision (see the handoff).
