// Floor measurement: a window with one Label and no Theme, to separate "Qt Quick + style" cost from page cost.
import QtQuick
import QtQuick.Controls

ApplicationWindow {
    width: 1100
    height: 720
    visible: true
    title: "T-0011 minimal"
    Label { anchors.centerIn: parent; text: "minimal" }
}
