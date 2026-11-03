// Teardown stress scene (dev-only, see main.rs): a scroll / cancel / churn storm against the artwork provider, then
// Qt.quit() after --exit-ms=<ms> (default 600) while requests are still in flight.
// --mode=storm (default): everything at once; nearly every request is cancelled before a worker starts it.
// --mode=scroll: only the grid jumping every 30 ms, so many more requests really run when the scene quits.
// Raw numbers are fine here: this file is a test fixture, not app UI (qml-ui.md applies to qml/).
import QtQuick

Window {
    id: win

    visible: true
    width: 480
    height: 640

    readonly property bool storm: argValue("mode", "storm") === "storm"

    function argValue(name, fallback) {
        const prefix = "--" + name + "="
        const arg = Qt.application.arguments.find(a => a.startsWith(prefix))
        return arg === undefined ? fallback : arg.substring(prefix.length)
    }

    // A grid of 200 000 cells that jumps around: every jump drops hundreds of in-flight requests (cancel storm).
    GridView {
        id: grid
        anchors.fill: parent
        cellWidth: 48
        cellHeight: 48
        model: 200000
        cacheBuffer: 0
        delegate: Image {
            width: 48
            height: 48
            asynchronous: true
            cache: false
            sourceSize: Qt.size(48, 48)
            source: "image://art/" + index
        }
        Timer {
            interval: win.storm ? 4 : 30
            running: true
            repeat: true
            onTriggered: grid.contentY = Math.max(0, Math.random() * (grid.contentHeight - grid.height))
        }
    }

    // Sources that change, clear and fail all the time.
    Repeater {
        model: win.storm ? 60 : 0
        delegate: Image {
            x: (index % 10) * 40
            y: Math.floor(index / 10) * 40
            width: 40
            height: 40
            asynchronous: true
            cache: false
            sourceSize: Qt.size(40, 40)
            Timer {
                interval: 1 + (index % 5)
                running: true
                repeat: true
                onTriggered: {
                    const r = Math.random()
                    parent.source = r < 0.2 ? "" : r < 0.3 ? "image://art/missing-" + index : "image://art/" + Math.floor(Math.random() * 100000)
                }
            }
        }
    }

    // Create and destroy images continuously.
    Loader {
        id: churn
        active: win.storm
        sourceComponent: Row {
            Repeater {
                model: 30
                delegate: Image {
                    width: 8
                    height: 8
                    asynchronous: true
                    cache: false
                    sourceSize: Qt.size(8, 8)
                    source: "image://art/" + (index + Math.floor(Math.random() * 1000))
                }
            }
        }
    }
    Timer {
        interval: 3
        running: win.storm
        repeat: true
        onTriggered: churn.active = !churn.active
    }

    Timer {
        interval: Math.max(50, parseInt(win.argValue("exit-ms", "600")))
        running: true
        onTriggered: Qt.quit()
    }
}
