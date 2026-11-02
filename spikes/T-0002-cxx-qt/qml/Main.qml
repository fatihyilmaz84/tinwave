import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Tw.Spike

ApplicationWindow {
    id: win
    visible: true
    width: Theme.windowMinWidth + 100
    height: Theme.windowMinHeight + 140
    minimumWidth: Theme.windowMinWidth
    minimumHeight: Theme.windowMinHeight
    title: "Tinwave spike T-0002"

    readonly property bool benchMode: bench.env("T2_BENCH") === "1"
    readonly property bool forward: bench.env("T2_FORWARD") !== "0"
    readonly property bool preIndicator: bench.env("T2_INDICATOR") === "pre"
    property bool taskActive: false
    property string taskText: "no task yet"

    TrackModel {
        id: tracks
        onTaskFinished: latencyUs => {
            win.taskActive = false
            win.taskText = "200 ms task done, posted to Qt thread in " + latencyUs + " us"
            if (win.benchMode) bench.log("task_post_latency_us=" + latencyUs)
        }
    }
    Counters { id: counters }
    Bench { id: bench }

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            spacing: Theme.spaceM
            ToolButton {
                text: "Run 200 ms task"
                enabled: !win.taskActive
                onClicked: { win.taskActive = true; tracks.runSlowTask(200) }
            }
            BusyIndicator { running: win.taskActive; visible: win.preIndicator || win.taskActive }
            Label { text: win.taskText; Layout.fillWidth: true; elide: Text.ElideRight }
        }
    }

    Item {
        id: frame
        anchors.fill: parent
        // Only for the T2_SHOT render check: grabToImage output is transparent, so give it the window colour.
        Rectangle { anchors.fill: parent; visible: bench.env("T2_SHOT") !== ""; color: win.palette.window }

    ListView {
        id: list
        anchors.fill: parent
        model: tracks
        reuseItems: true
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}

        delegate: TrackRow {
            onHoverChanged: (index, entered) => { if (win.forward) counters.hover(index, entered) }
        }

        function reportViewport() {
            if (!win.forward) return
            const first = indexAt(0, contentY + 1)
            const last = indexAt(0, contentY + height - 1)
            if (first >= 0) counters.viewport(first, last >= 0 ? last : first)
        }
        onContentYChanged: reportViewport()
    }
    }

    // ---- frame accounting (first frame + per-phase stats) ----
    property bool started: false
    Connections {
        target: win
        function onFrameSwapped() {
            bench.frameSwapped()
            if (!win.started) {
                win.started = true
                bench.firstFrame()
                counters.pageVisible("library", true)
                if (bench.env("T2_SHOT") !== "") shotTimer.start()
                if (win.benchMode) { bench.log("dpr=" + Screen.devicePixelRatio + " style=" + bench.env("T2_STYLE")); runStep() }
            }
        }
    }

    // ---- benchmark script (T2_BENCH=1): idle -> flick -> traverse -> tasks -> counters cost -> quit ----
    property int step: 0
    property real rowsPerSecFlick: 200

    NumberAnimation {
        id: scrollAnim
        target: list
        property: "contentY"
        easing.type: Easing.Linear
        onFinished: stepTimer.start()
    }
    Timer { id: stepTimer; interval: 1; onTriggered: win.runStep() }
    Timer { id: delayed; onTriggered: win.runStep() }
    Timer { id: blockingKick; interval: 300; onTriggered: tracks.runSlowTaskBlocking(200) }

    function wait(ms) { delayed.interval = ms; delayed.start() }
    function scrollTo(y, ms) { scrollAnim.to = y; scrollAnim.duration = ms; scrollAnim.restart() }
    function maxY() { return list.contentHeight - list.height }

    property int trial: 0
    function runStep() {
        switch (step++) {
        case 0: bench.phase("startup_settle"); wait(2500); break
        case 1: bench.phase("idle_steady"); wait(3000); break
        case 2: bench.log("after idle: " + tracks.stats()); bench.phase("flick_200rows_per_s_cold")
                scrollAnim.from = 0; scrollTo(Theme.rowHeight * 200 * 5, 5000); break
        case 3: bench.phase("settle_1s"); wait(1500); break
        case 4: bench.log("counters after flick: " + counters.snapshot() + " | " + tracks.stats())
                bench.phase("flick_200rows_per_s_warm")
                scrollAnim.from = 0; scrollTo(Theme.rowHeight * 200 * 5, 5000); break
        case 5: bench.phase("traverse_down_3s_33k_rows_per_s")
                scrollAnim.from = list.contentY; scrollTo(maxY(), 3000); break
        case 6: bench.phase("traverse_up_3s")
                scrollAnim.from = list.contentY; scrollTo(0, 3000); break
        case 7: bench.phase("settle_2s"); wait(2000); break
        case 8: bench.log("after traverse: " + tracks.stats())
                bench.phase("flick_with_async_task_x3")
                scrollAnim.from = 0; scrollTo(Theme.rowHeight * 200 * 6, 6000)
                trial = 0; taskKick.start(); break
        case 9: bench.phase("flick_with_blocking_task_control")
                scrollAnim.from = 0; scrollTo(Theme.rowHeight * 200 * 2, 2000)
                blockingKick.start(); break
        case 10: bench.phase("counter_call_cost")
                var n = 200000
                var t0 = Date.now()
                for (var i = 0; i < n; ++i) counters.hover(i & 1023, (i & 1) === 0)
                var t1 = Date.now()
                for (var j = 0; j < n; ++j) counters.viewport(j & 1023, (j & 1023) + 20)
                var t2 = Date.now()
                for (var k = 0; k < n; ++k) list.indexAt(0, (k & 1023) * 40 + 1)
                var t3 = Date.now()
                bench.log("qml_to_rust_call_ns hover=" + ((t1 - t0) * 1e6 / n).toFixed(0)
                          + " viewport=" + ((t2 - t1) * 1e6 / n).toFixed(0)
                          + " | qml indexAt only=" + ((t3 - t2) * 1e6 / n).toFixed(0) + " (n=" + n + ")")
                bench.log("debounce selftest start: " + counters.snapshot())
                counters.hover(7, true); wait(300); break          // short hover: must NOT count
        case 11: counters.hover(7, false); counters.hover(9, true); wait(1100); break  // >= 800 ms: must count once
        case 12: counters.hover(9, false)
                bench.log("debounce selftest end (expect artwork_hovers +1): " + counters.snapshot())
                wait(700); break    // viewport debouncer settles, then final numbers
        case 13: counters.pageVisible("library", false)
                bench.log("final counters: " + counters.snapshot() + " | " + tracks.stats())
                bench.report()
                Qt.quit()
                break
        }
    }
    // three 200 ms tasks, one every 1.5 s, during a flick
    Timer { id: taskKick; interval: 700; repeat: true
            onTriggered: { win.taskActive = true; tracks.runSlowTask(200); if (++win.trial >= 3) stop() } }

    // T2_SHOT=<file.png>: render check, scroll a bit so rows come from the middle of the model, save, quit.
    Timer { id: shotTimer; interval: 1500
            onTriggered: { bench.log("shot: positioning"); list.positionViewAtIndex(54321, ListView.Beginning); shotGrab.start() } }
    Timer { id: shotGrab; interval: 1200
            onTriggered: { bench.log("shot: grabbing " + frame.width + "x" + frame.height + " ok=" + frame.grabToImage(function (r) { bench.log("saved=" + r.saveToFile(bench.env("T2_SHOT"))); Qt.quit() })) } }
    Timer { interval: 90000; running: win.benchMode; onTriggered: { bench.log("WATCHDOG quit"); Qt.quit() } }
}
