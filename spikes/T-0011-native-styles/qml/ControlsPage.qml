// Reserved: Main.qml carries the controls page inline for the spike. Kept so the module has a second file,
// and used as the "shape" reference for a Settings page that follows the Theme override rules.
import QtQuick
import QtQuick.Controls
import Tinwave.Theme

Pane {
    // Settings > Appearance prototype: scheme override + accent override.
    Column {
        spacing: Theme.s
        ComboBox {
            model: ["Follow system", "Light", "Dark"]
            onActivated: i => Theme.setSchemeOverride(["system", "light", "dark"][i])
            Accessible.name: "Theme"
        }
    }
}
