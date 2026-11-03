// Teardown stress scene (dev-only, see main.rs): a scroll / cancel / churn storm against the artwork provider, then
// Qt.quit() while requests are still in flight. The quit waits on conditions, not only on the clock (T-0071): on a
// loaded machine the thread that hands requests to the provider can be starved for longer than any fixed time, and a
// quit before the provider saw a request or a cancellation tests nothing. In order:
//   1. the "canary" (see below) is answered: the provider is receiving requests; --exit-ms=<ms> (default 600) starts;
//   2. --exit-ms has passed AND the grid has jumped at least minJumps times since (each jump drops its requests).
// Then Qt.quit(), with the other requests still queued, running or being cancelled. What the scene cannot see is
// whether the provider has already handled those cancellations (the thread that does is saturated by a storm), so a
// run that quit before any was counted is possible on a starved machine; the test treats it as such (cancels = 0).
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
            onTriggered: {
                grid.contentY = Math.max(0, Math.random() * (grid.contentHeight - grid.height))
                if (win.armed) {
                    win.jumps++
                    win.maybeQuit()
                }
            }
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

    // The quit sequence (see the top of the file). The canary asks for a thumbnail above the edge limit, which the
    // provider refuses at once on the thread that hands it requests (no worker involved): its Image.Error means that
    // thread is running and has taken a request.
    readonly property int minJumps: 3
    property bool armed: false
    property bool timeUp: false
    property int jumps: 0

    function maybeQuit() {
        if (timeUp && jumps >= minJumps) {
            Qt.quit()
        }
    }

    Image {
        id: canary
        visible: false
        asynchronous: true
        cache: false
        sourceSize: Qt.size(100000, 100000)
        source: "image://art/canary"
        onStatusChanged: {
            if (status === Image.Error && !win.armed) {
                win.armed = true
                quitTimer.start()
            }
        }
    }
    Timer {
        id: quitTimer
        interval: Math.max(50, parseInt(win.argValue("exit-ms", "600")))
        onTriggered: {
            win.timeUp = true
            win.maybeQuit()
        }
    }
}
