// Test-only root (loaded through TINWAVE_MAIN_QML in debug builds by tests/contrast_qml.rs): runs the contrast gate of
// design-system.md 3.1 on the real Theme and prints its result as `PROBE name=value` lines, then quits. No window is made.
import QtQuick
import Tinwave.Desktop

QtObject {
    id: probe

    function say(name, value) { console.log("PROBE " + name + "=" + value) }

    property Timer finish: Timer {
        interval: 10
        running: true
        onTriggered: {
            // The WCAG maths the gate stands on: black on white is 21:1, a colour on itself 1:1.
            probe.say("blackOnWhite", Theme.contrast(Qt.rgba(0, 0, 0, 1), Qt.rgba(1, 1, 1, 1)).toFixed(2))
            probe.say("whiteOnWhite", Theme.contrast(Qt.rgba(1, 1, 1, 1), Qt.rgba(1, 1, 1, 1)).toFixed(2))
            probe.say("accents", Theme.checkAccents.length)
            const result = Theme.selfCheck()
            probe.say("failures", result.failures)
            probe.say("rowCount", result.rows.length)
            for (const row of result.rows) probe.say("row", row)
            Qt.quit()
        }
    }
}
