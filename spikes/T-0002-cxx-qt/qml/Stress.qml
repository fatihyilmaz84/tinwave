import QtQuick
import QtQuick.Controls
import Tw.Spike

// T-0041 stress: scroll / cancel / teardown storm against the real Rust-backed async image provider.
// T2_MODE=stress, T2_EXIT_MS=<ms until Qt.quit(), default 600>.
ApplicationWindow {
    id: win
    visible: true
    width: 480; height: 640
    Bench { id: bench }

    GridView {
        anchors.fill: parent
        id: grid
        cellWidth: 48; cellHeight: 48
        model: 200000
        cacheBuffer: 0
        delegate: Image {
            width: 48; height: 48
            asynchronous: true; cache: false
            sourceSize: Qt.size(48, 48)
            source: "image://art/" + index
        }
    }
    Repeater {
        model: 60
        delegate: Image {
            x: (index % 10) * 40; y: Math.floor(index / 10) * 40; width: 40; height: 40
            asynchronous: true; cache: false
            sourceSize: Qt.size(40, 40)
            Timer {
                interval: 1 + (index % 5); running: true; repeat: true
                onTriggered: parent.source = (Math.random() < 0.2) ? "" : "image://art/" + Math.floor(Math.random() * 100000)
            }
        }
    }
    Loader {
        id: churn
        active: true
        sourceComponent: Row {
            Repeater { model: 30; delegate: Image { width: 8; height: 8; asynchronous: true; cache: false; sourceSize: Qt.size(8, 8); source: "image://art/" + (index + Math.floor(Math.random() * 1000)) } }
        }
    }
    Timer { interval: 3; running: true; repeat: true; onTriggered: churn.active = !churn.active }
    Timer {
        interval: 4; running: true; repeat: true
        onTriggered: grid.contentY = Math.max(0, Math.random() * (grid.contentHeight - grid.height))
    }
    Timer {
        interval: Math.max(50, parseInt(bench.env("T2_EXIT_MS")) || 600); running: true
        onTriggered: Qt.quit()
    }
}
