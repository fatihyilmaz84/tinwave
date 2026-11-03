// Size and status scene for the artwork provider (dev-only, see main.rs): loads one Image per --images=<key>@<w>x<h>,...
// entry (w or h may be 0 = "not set", like a one-edge sourceSize), waits until every Image is Ready or Error, prints one
// line per Image and quits. Used by tests/image_provider.rs, which reads the lines from the log.
//   SIZE <index> <key>@<w>x<h> status=<Ready|Error> w=<implicitWidth> h=<implicitHeight>
//   SIZES_DONE
// Exits with code 3 after 20 s if an Image never settles.
// Raw numbers are fine here: this file is a test fixture, not app UI (qml-ui.md applies to qml/).
import QtQuick

Window {
    id: win

    visible: true
    width: 400
    height: 400

    property int settled: 0

    function argValue(name, fallback) {
        const prefix = "--" + name + "="
        const arg = Qt.application.arguments.find(a => a.startsWith(prefix))
        return arg === undefined ? fallback : arg.substring(prefix.length)
    }

    readonly property var entries: argValue("images", "").split(",").filter(e => e.length > 0).map(e => {
        const at = e.lastIndexOf("@")
        const dims = e.substring(at + 1).split("x")
        return { key: e.substring(0, at), w: parseInt(dims[0]), h: parseInt(dims[1]), text: e }
    })

    Flow {
        anchors.fill: parent
        Repeater {
            model: win.entries
            delegate: Image {
                id: img

                required property int index
                required property var modelData
                property bool counted: false

                asynchronous: true
                cache: false
                sourceSize: Qt.size(modelData.w, modelData.h)
                source: "image://art/" + modelData.key
                onStatusChanged: {
                    if (counted || (status !== Image.Ready && status !== Image.Error))
                        return
                    counted = true
                    console.log("SIZE " + index + " " + modelData.text + " status=" + (status === Image.Ready ? "Ready" : "Error")
                                + " w=" + implicitWidth + " h=" + implicitHeight)
                    if (++win.settled === win.entries.length) {
                        console.log("SIZES_DONE")
                        Qt.quit()
                    }
                }
            }
        }
    }

    Timer {
        interval: 20000
        running: true
        onTriggered: Qt.exit(3)
    }
}
