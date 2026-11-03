// Test-only root (loaded through TINWAVE_MAIN_QML in debug builds by tests/icon_qml.rs): shows a TwIcon in a real (offscreen)
// window and prints what its image did, then quits. Icon names here are not scanned by the build's icon check (it reads qml/
// only), so the file may name any vendored icon.
import QtQuick
import QtQuick.Window
import Tinwave.Desktop

QtObject {
    id: probe

    function say(name, value) { console.log("PROBE " + name + "=" + value) }

    property Window window: Window {
        visible: true
        width: 120
        height: 80

        TwIcon {
            id: icon
            name: "menu"
            size: Theme.iconSizeDense
            color: Theme.accentText
        }
        TwIcon {
            id: unnamed
            x: 40
        }
    }

    property Timer finish: Timer {
        interval: 300
        running: true
        onTriggered: {
            const image = icon.children[0]
            probe.say("source", image.source)
            probe.say("status", image.status === Image.Ready ? "ready" : "status-" + image.status)
            probe.say("sourceWidth", image.sourceSize.width)
            probe.say("sourceHeight", image.sourceSize.height)
            probe.say("implicitWidth", icon.implicitWidth)
            probe.say("implicitHeight", icon.implicitHeight)
            probe.say("unnamedSource", "[" + unnamed.children[0].source + "]")
            probe.say("unnamedImplicitWidth", unnamed.implicitWidth)
            Qt.quit()
        }
    }
}
