# Vendored icons

[Lucide](https://lucide.dev) icons (ISC; the Feather-derived ones also MIT), version in `VERSION`. Licence text:
[LICENSE-lucide.txt](LICENSE-lucide.txt). It is the **notices source** for Lucide: THIRD-PARTY-NOTICES (T-0042) must
include it verbatim, and the build also compiles it into the QML module's resources next to the icons.

- Only icons the QML uses live here. `build.rs` fails the build if a file here is not named in QML (or the other way round).
  QML names an icon only as a string literal: `iconName: "x"`, `TwIcons.source("x")`, or `name: "x"` inside `TwIcon { }`.
- Each SVG is the Lucide original with one change: `stroke-width="2"` became `"1.75"` (`Theme.iconStroke`, design-system §3.4).
- Add, bump or refresh with `powershell -File tools/vendor-icons.ps1 [-Add a,b] [-Version x.y.z]`; do not edit the SVGs by hand.
- Rendered through Qt's SVG plugins (`qsvg`, `qsvgicon`); the Windows deploy recipe (ADR 0008) already ships them.
