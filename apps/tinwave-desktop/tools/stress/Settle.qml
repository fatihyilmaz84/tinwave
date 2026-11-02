// Deterministic scene for the "exactly one finished() per response" test (dev-only, see main.rs): 64 images, 8 of
// them for keys without artwork, quits once every image reached Ready or Error, or exits with code 3 after 20 s.
import QtQuick

Window {
    id: win

    visible: true
    width: 400
    height: 400

    readonly property int total: 64
    property int settled: 0

    Flow {
        anchors.fill: parent
        Repeater {
            model: win.total
            delegate: Image {
                property bool counted: false

                width: 32
                height: 32
                asynchronous: true
                cache: false
                sourceSize: Qt.size(32, 32)
                // Keys 0..15 (except the missing ones) repeat, so the cache is exercised too.
                source: index % 8 === 7 ? "image://art/missing-" + index : "image://art/" + (index % 16)
                onStatusChanged: {
                    if (!counted && (status === Image.Ready || status === Image.Error)) {
                        counted = true
                        win.settled++
                        if (win.settled === win.total)
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
