// TwIcon (design-system.md sections 3.4 and 5): a Lucide glyph tinted with a colour token, for places where a native
// control has no icon slot. Native controls (ToolButton, ItemDelegate, Button...) keep their own `icon.source` /
// `icon.color` / `icon.width` instead.
//
// The SVG carries the stroke width (1.75, `Theme.iconStroke`; baked in by tools/vendor-icons.ps1) and is rasterised at the
// size x device pixel ratio, so it stays crisp at 100 / 150 / 200 % scaling. The glyph is decorative: the surrounding
// control provides the accessible name.
import QtQuick
import QtQuick.Controls.impl
import Tinwave.Desktop

Item {
    id: root

    // Lucide icon name, e.g. "menu" (a file in icons/). Always a string literal, see TwIcons.
    property string name

    // Logical size: `Theme.iconSize` (20) or `Theme.iconSizeDense` (16, dense rows).
    property int size: Theme.iconSize

    // Tint: `Theme.text` / `Theme.textMuted` (or `Theme.accentText` for an accent glyph).
    property color color: Theme.text

    implicitWidth: size
    implicitHeight: size
    Accessible.ignored: true

    IconImage {
        anchors.fill: parent
        source: root.name === "" ? "" : TwIcons.source(root.name)
        sourceSize: Qt.size(root.size, root.size)
        color: root.color
    }
}
