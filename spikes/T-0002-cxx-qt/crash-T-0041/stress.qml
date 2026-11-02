import QtQuick
import QtQuick.Window

// Scroll / cancel / teardown storm for the async image provider.
Window {
    id: win
    visible: true
    width: 480; height: 640
    property bool storm: true

    GridView {
        id: grid
        anchors.fill: parent
        cellWidth: 48; cellHeight: 48
        model: win.storm ? 200000 : 0
        cacheBuffer: 0
        visible: cfg.mode !== "toggle"
        delegate: Image {
            width: 48; height: 48
            asynchronous: true
            cache: false
            sourceSize: Qt.size(48, 48)
            source: cfg.mode === "noimg" ? "" : "image://art/" + index
        }
    }

    // many Images whose source flips between ids / empty: request, cancel, re-request on the same item
    Repeater {
        model: win.storm ? 60 : 0
        delegate: Image {
            x: (index % 10) * 40; y: Math.floor(index / 10) * 40; width: 40; height: 40
            asynchronous: true; cache: false
            sourceSize: Qt.size(40, 40)
            Timer {
                interval: 1 + (index % 5); running: win.storm && cfg.mode !== "scroll"; repeat: true
                onTriggered: parent.source = (cfg.mode === "noimg" || Math.random() < 0.2) ? "" : "image://art/" + Math.floor(Math.random() * 100000)
            }
        }
    }

    // churn whole delegates in and out of existence
    Loader {
        id: churn
        active: win.storm
        sourceComponent: Row {
            Repeater { model: 30; delegate: Image { width: 8; height: 8; asynchronous: true; cache: false; sourceSize: Qt.size(8, 8); source: cfg.mode === "noimg" ? "" : "image://art/" + (index + Math.floor(Math.random()*1000)) } }
        }
    }
    Timer { interval: 3; running: win.storm && cfg.mode !== "scroll"; repeat: true; onTriggered: churn.active = !churn.active }

    Timer {
        interval: cfg.tick; running: win.storm && cfg.mode !== "toggle"; repeat: true
        onTriggered: grid.contentY = Math.max(0, Math.random() * (grid.contentHeight - grid.height))
    }
    Timer { interval: cfg.exitMs; running: true; onTriggered: { if (cfg.drain > 0) { win.storm = false; churn.active = false; drainTimer.start() } else Qt.quit() } }
    Timer { id: drainTimer; interval: cfg.drain; onTriggered: Qt.quit() }
}
