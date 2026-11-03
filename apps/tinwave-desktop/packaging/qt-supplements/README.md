# qt-supplements

License texts of Qt-internal third-party components for which the Qt kit's documentation has no attribution page
(`New-ThirdPartyNotices.ps1` uses the page when it exists and this folder otherwise; it fails if neither exists).
Each file is copied byte for byte from the Qt source tag of the version shipped. File name = the component id in lower case
without separators.

| File | Component | Source (tag v6.12.0) |
|---|---|---|
| `xsvg.txt` | XSVG (arc handling in Qt Gui), HPND-sell-variant | `qtbase/src/gui/util/LICENSE.XSVG.txt` (https://code.qt.io/cgit/qt/qtbase.git/plain/src/gui/util/LICENSE.XSVG.txt?h=v6.12.0) |

When the Qt kit is upgraded and the generator reports a new component without a page, add its `LICENSE*` file from the new
Qt tag here and a row in this table.
