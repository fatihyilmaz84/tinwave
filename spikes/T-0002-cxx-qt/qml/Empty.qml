import QtQuick
import QtQuick.Controls
import Tw.Spike

// Baseline: the smallest native Qt Quick Controls window, to separate "Qt floor" from "our list".
ApplicationWindow {
    id: win
    visible: true
    width: Theme.windowMinWidth
    height: Theme.windowMinHeight
    title: "Tinwave spike T-0002 (empty baseline)"

    Bench { id: bench }

    Label { anchors.centerIn: parent; text: "empty" }

    property bool started: false
    Connections {
        target: win
        function onFrameSwapped() {
            bench.frameSwapped()
            if (!win.started) {
                win.started = true
                bench.firstFrame()
                bench.phase("idle")
                idleTimer.start()
            }
        }
    }
    Timer {
        id: idleTimer
        interval: 3000
        onTriggered: { bench.report(); Qt.quit() }
    }
    Timer { interval: 20000; running: true; onTriggered: Qt.quit() }
}
