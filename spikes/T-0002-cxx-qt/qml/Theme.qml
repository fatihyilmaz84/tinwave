pragma Singleton
import QtQuick

// Spike-sized subset of the design-system tokens (docs/design/design-system.md section 3).
// The only file in the spike allowed to contain raw numbers/colors.
QtObject {
    readonly property bool dark: Qt.styleHints.colorScheme === Qt.ColorScheme.Dark

    // space (4 px grid)
    readonly property int spaceXs: 4
    readonly property int spaceS: 8
    readonly property int spaceM: 12
    readonly property int spaceL: 16

    // sizes
    readonly property int rowHeight: 40
    readonly property int artSmall: 32
    readonly property int windowMinWidth: 880
    readonly property int windowMinHeight: 560
    readonly property int durationColumn: 56
    readonly property int artistColumn: 200

    // radius
    readonly property int radiusArtSmall: 4

    // color (semantic subset)
    readonly property color raised: dark ? "#202023" : "#FFFFFF"
    readonly property color textMuted: dark ? "#A6A6AF" : "#5D5D66"

    // type
    readonly property int fontCaption: Qt.application.font.pixelSize - 2

    // motion (reduced motion not wired in the spike)
    readonly property int durationFast: 120
}
