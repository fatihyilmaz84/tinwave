# Windows packaging

Scripted, repeatable packaging of `tinwave-desktop.exe` into one self-contained folder, with the legal files that Qt's LGPL-3.0
and the other third-party licenses require ([ADR 0008](../../../docs/adr/0008-cxx-qt-bridge.md) decision 3, section 3.5 and
"Licenses"). PowerShell only (Windows PowerShell 5.1 or 7); nothing is installed or changed outside the process.

```
$env:QMAKE = 'C:\Qt\6.12.0\msvc2022_64\bin\qmake.exe'      # the documented build environment; Qt need not be on PATH
powershell -File apps\tinwave-desktop\packaging\package.ps1  # -> target\package\tinwave-desktop-windows-x64\
```

Options: `-OutDir`, `-QtDir` (instead of `QMAKE`), `-CrtDir` (MSVC redistributable folder), `-SkipBuild` (use the existing
`target\release\tinwave-desktop.exe`), `-NoSmoke` (headless machines: no window is opened), `-Release` (a pending D-0007 fails
the package; see "D-0007" below).

No network at package time: `cargo build --frozen` (Cargo.lock honoured, nothing fetched) and `cargo metadata --locked --offline`.
`-OutDir` is wiped and rebuilt only when it is empty or carries the marker file `.tinwave-package`, which this script writes
into every folder it creates; any other non-empty folder is refused, whatever it contains.

## What `package.ps1` does

1. `cargo build --release --frozen -p tinwave-desktop --bin tinwave-desktop`. Only that binary: `--workspace` also builds the
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
| `gpl-only` | a bundled DLL is a Qt test module or has no SBOM entry, or the kit's SBOM says its license has no LGPL/permissive option; the exe contains the name of a GPL-only module (`QuickTestUtils`, `QuickControlsTestUtils`, any `TestUtils`). Plugin DLLs map to the SBOM by exact name (a table for the few that differ, e.g. `qwindows` -> `QWindowsIntegrationPlugin`); a name that is only a prefix matches nothing and so fails closed |
| `link-libs` | a build script asked the linker for a Qt library that is not in the kit's SBOM, is GPL-only, or is linked `static=`. Reads the `cargo:rustc-link-lib` / `cargo::rustc-link-lib` lines of every `target\release\build\*\output`; fails when there are none (nothing built, nothing verifiable). In this kit the GPL-only modules are **static libraries**, so linking one would put GPL-only code into the exe without any DLL appearing: this is the authoritative list of what was linked, whichever crate's build script asked for it |
| `qt-unmodified` | a file in the bundle (other than the exe, the MSVC runtime and the legal files) is not byte-identical (SHA-256) to its counterpart in the Qt kit (`bin\` for root DLLs, `plugins\` and `qml\` for folders), or has none |
| `only-app-exe` | the bundle holds any `.exe` other than `tinwave-desktop.exe`, or `.pdb/.lib/.exp/...` files |
| `closure` | any DLL/exe imports a DLL that is neither in the bundle root nor a Windows system DLL; the MSVC runtime must be in the root |
| `legal` | `THIRD-PARTY-NOTICES.txt`, `SOURCE-OFFER-Qt.txt` or `licenses\Qt-LGPL-3.0-and-GPL-3.0.txt` is missing, or the last one lacks the LGPL-3.0 or GPL-3.0 text; the notices carry FreeType without the FTL acknowledgement sentence, or the IJG license without the IJG sentence, or have no "Rust standard library" section |
| `source-offer` | the offer does not link `download.qt.io/archive/qt/<maj.min>/<ver>/submodules/<repo>-everywhere-src-<ver>.tar.xz` for **every** Qt repository (`qtbase`, `qtdeclarative`, `qtsvg`, ... taken from the SBOM file each shipped Qt module is listed in), does not point at the release archive folder, links the pruned `official_releases`, or has an unfilled `{{placeholder}}` |
| `d0007` | see "D-0007" below; reported as `PENDING`, which is not a failure, until the PO has decided |

`smoke-test.ps1` starts the bundle with `PATH` reduced to Windows and the `QT_*`/`QML_*` variables cleared, waits for a visible
window titled `Tinwave`, checks that every loaded Qt DLL (and `qwindows.dll`, the FluentWinUI3 plugin) comes from the bundle
folder, then kills the process. It fails on early exit, on no window within 30 s, and on Qt errors in stderr.

`test-packaging.ps1` proves the checks have teeth: it breaks a copy of a good bundle in many ways (GPL-only DLL, GPL-only string
in the exe, a GPL-only or static Qt library requested through `rustc-link-lib`, a modified or foreign Qt file, a Qt repository
missing from the source offer, the pruned download link, missing FreeType / IJG sentences, no dynamic Qt, `image-stress.exe`,
missing runtime, missing `Qt6Core.dll`, missing source offer, `-Release` with D-0007 pending, a stale bundle after D-0007,
a smoke test with a missing DLL while Qt is on the caller's PATH that must end in `-1073741515` (STATUS_DLL_NOT_FOUND), an
`-OutDir` without the marker, ...) and asserts exit code 1 plus the FAIL line; it also proves the D-0007 switch flips. Run it
after any change to these scripts.

## D-0007: copyright holder and source-offer contact (pending PO decision)

Who holds the copyright of Tinwave's own code and who answers the Qt written source offer are PO decisions
(`work/decisions/D-0007-licence-holder-and-source-offer.md`). Nothing here invents either. `release-identity.json` holds
`null` for both, and until it is filled in:

* `THIRD-PARTY-NOTICES.txt` says `Copyright holder: [D-0007 PENDING: copyright holder not decided]` and `SOURCE-OFFER-Qt.txt`
  says `Contact for this offer: [D-0007 PENDING: source-offer contact not decided]`;
* `check-bundle` prints `PENDING d0007  D-0007 pending (...)`, listing what is open (holder, contact, `LICENSE-MIT` and
  `LICENSE-APACHE` at the repository root), and still exits 0, so everyday packaging is not blocked;
* `package.ps1 -Release` (and `check-bundle.ps1 -Release`) turns every pending item into a FAIL: that is the command for a
  bundle that is going to be published.

How the switch flips once the PO has decided: (1) put the holder and the contact into `release-identity.json`
(`copyrightHolder`, `offerContact`), (2) add `LICENSE-MIT` and `LICENSE-APACHE` at the repository root (`package.ps1` then copies
them to `licenses\Tinwave-LICENSE-*.txt`), (3) run `package.ps1 -Release`, refresh the committed `THIRD-PARTY-NOTICES.txt`
and commit. From then on a bundle that still carries a `[D-0007 PENDING` marker, or does not contain the values from the file,
fails the check as stale. No script change is needed.

## Legal files in the bundle

| File | Source |
|---|---|
| `THIRD-PARTY-NOTICES.txt` | `New-ThirdPartyNotices.ps1`: Tinwave's license, the Qt LGPL-3.0 statement, the Qt modules and plugins actually in the folder, third-party code inside Qt with full license texts, the MSVC runtime, the Rust crates and their license texts, the Rust standard library and the crates it vendors, the FreeType and IJG acknowledgement sentences, and every `LICENSE*` under `apps\tinwave-desktop\icons\` when that folder exists (the Lucide ISC text from T-0043) |
| `licenses\Qt-LGPL-3.0-and-GPL-3.0.txt` | `<Qt>\Licenses\LICENSE`, Qt's own file holding both texts |
| `licenses\Apache-2.0.txt` | `license-texts\Apache-2.0.txt` (Tinwave is `MIT OR Apache-2.0`) |
| `SOURCE-OFFER-Qt.txt` | `templates\SOURCE-OFFER-Qt.txt` with the kit's version, one source archive per Qt repository that the shipped Qt files come from (read from the SBOM), and the offer contact (D-0007) filled in |
| `licenses\Tinwave-LICENSE-MIT.txt`, `...-APACHE.txt` | `LICENSE-MIT` / `LICENSE-APACHE` at the repository root, once D-0007 adds them |
| `.tinwave-package` | marker file: `package.ps1` deletes this folder on the next run only because it is here |

`packaging\THIRD-PARTY-NOTICES.txt` is a committed copy of the generated file so a reviewer can read it; `package.ps1` warns when
the generated one differs. Refresh it with `New-ThirdPartyNotices.ps1 -Bundle <bundle>` (default output is that file) and commit.

### Where the notice data comes from (all installed, nothing downloaded)

- **Rust crates**: `cargo metadata --locked --offline --filter-platform x86_64-pc-windows-msvc`; the normal-dependency closure of the
  package, minus proc-macro crates (they run in the compiler). License texts are the files inside each crate's registry
  source. A crate that ships none (`cxx-qt`, `cxx-qt-lib`) gets the canonical text of each SPDX id of its expression from
  `license-texts\` (MIT with the holder from the crate's `SPDX-FileCopyrightText:` lines, e.g. `2021-2026 Klaralvdalens
  Datakonsult AB, a KDAB Group company`, else the `Cargo.toml` authors; a crate that names nobody, or a missing canonical text,
  fails the run). MPL-2.0 crates (Symphonia, D-0003) get a source/unmodified statement.
- **Rust standard library** (statically linked; MIT OR Apache-2.0 plus the vendored crates): the list is **derived from the
  link**, not typed. `target\release\tinwave_desktop.pdb` names every `lib<crate>-<hash>.rlib` the linker took objects from; the
  ones that exist with that hash in the pinned toolchain's sysroot (`rustc --print sysroot`, i.e. `rust-toolchain.toml`) are std's.
  Today that is alloc, compiler_builtins, core, panic_unwind, std (in-tree) plus hashbrown and rustc-demangle (vendored). gimli,
  addr2line, miniz_oxide and friends are not linked on x86_64-pc-windows-msvc (backtraces use dbghelp) and are listed as not
  linked. The notices and texts come from the toolchain itself: `<sysroot>\share\doc\rust\COPYRIGHT-library.html` (the Rust
  project's own list of the std source tree and its vendored crates, each with its LICENSE files) and `<sysroot>\share\doc\rust\
  licenses\*.txt` (SPDX texts for the in-tree entries: Apache-2.0, MIT, Unicode-3.0, BSD-2-Clause). The generator fails if the pdb
  is missing or names no std rlib of the toolchain, or if a linked vendored crate has no text there.
- **Qt**: the SPDX SBOMs in `<kit>\sbom` decide which modules and plugins are under which license and which third-party
  components belong to which module (only modules whose DLLs are in the bundle count). The text of each component is Qt's own
  attribution page from `<Qt>\Docs\Qt-<ver>\*\*-attribution-*.html`, which is `qtattributionsscanner`'s qdoc output with the
  license text embedded. `qtattributionsscanner.exe` itself is in the kit but scans `qt_attribution.json` files that exist only
  in Qt *source* trees (a binary kit has none for qtbase/qtdeclarative), so it cannot be run here. Components Qt publishes no
  page for come from `qt-supplements\` (today only XSVG; see the provenance note in `qt-supplements\README.md`: its first line lost a
  character and could not be re-copied offline, so the notice also prints the copyright line from Qt's SBOM above it).
- **FreeType / IJG sentences**: "Portions of this software are copyright (c) <year> The FreeType Project (www.freetype.org). All
  rights reserved." (the year is the newest copyright year on the FreeType attribution pages, i.e. of the version Qt bundles) and
  "This software is based in part on the work of the Independent JPEG Group." are emitted when those components are in the
  bundle, and `check-bundle` fails without them.
- The bundled libraries inside Qt (zlib, PCRE2, libpng, libjpeg, FreeType, HarfBuzz) are mapped to Qt modules by a table in
  the generator, guarded by a check of `config_qtbase.summary` ("Using system X ... no").

The generator **fails** instead of skipping: an SBOM component with no page and no supplement, a crate with no license text,
a Qt DLL with no SBOM entry. Needs the Qt kit's `Documentation`, `Licenses` and SBOM files (all in the standard installer) and the Rust toolchain's `share\doc\rust` folder (part of the rustc component).

## Upgrading Qt

Re-run `package.ps1` and `test-packaging.ps1`. New components appear as generator errors: add the missing `LICENSE*` file to
`qt-supplements\` (from the new Qt tag) or extend the bundled-library table. Update `SOURCE-OFFER-Qt.txt` only through the
template; the version is substituted. Check the trimmed bundle still starts (the smoke test does).

## Not covered

macOS and Linux packaging, an installer, code signing, update delivery. The source offer names Qt's archive as the place for the
exact Qt version; a Tinwave-side contact for the offer is a project decision (D-0007, above).
