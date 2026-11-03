# qt-supplements

License texts of Qt-internal third-party components for which the Qt kit's documentation has no attribution page
(`New-ThirdPartyNotices.ps1` uses the page when it exists and this folder otherwise; it fails if neither exists).
File name = the component id in lower case without separators.

| File | Component | Origin (Qt tag v6.12.0) |
|---|---|---|
| `xsvg.txt` | XSVG (arc handling in Qt Gui), HPND-sell-variant | `qtbase/src/gui/util/LICENSE.XSVG.txt` (https://code.qt.io/cgit/qt/qtbase.git/plain/src/gui/util/LICENSE.XSVG.txt?h=v6.12.0) |

## Provenance of `xsvg.txt` (read this before relying on the file)

* The text was copied from the URL above while T-0042 was built. It is **not** a verified byte-exact copy: its first line reads
  `Copyright  2002 USC/Information Sciences Institute` with two spaces, i.e. a character (almost certainly the copyright sign)
  was lost on the way into the repository. The rest of the file is the HPND-sell-variant permission text.
* Packaging must not need the network and no Qt 6.12.0 source tree is installed on the build machines (the Qt kit contains only
  binaries, headers, SBOMs and documentation). A byte-exact copy therefore could not be made offline for T-0051.
* What is authoritative for the copyright line is the Qt kit's own SBOM (`<kit>\sbom\qtbase-6.12.0.spdx.json`, package
  `Gui_Attribution_xsvg`): `Copyright 2002 USC/Information Sciences Institute`. `New-ThirdPartyNotices.ps1` prints that line
  directly above this file's text, so the shipped notice carries the correct copyright statement and the full permission text.
* SHA-256 of the committed file: `3a946598a9f674d44df764279ca0b85b5d6faba08f3bc75d8e0fcf6cd729a9bd` (1185 bytes), recorded so that a refresh is visible in review.

To replace it with a verified copy: with network access, download `LICENSE.XSVG.txt` from the tag above (or take it from an
extracted `qtbase-everywhere-src-6.12.0.tar.xz`), overwrite `xsvg.txt` byte for byte, update the hash here, and re-run
`package.ps1`. Nothing else needs to change.

When the Qt kit is upgraded and the generator reports a new component without a page, add its `LICENSE*` file from the new
Qt tag here and a row in this table.
