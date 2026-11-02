// Window shell (design-system.md section 4, canvas "Library - Windows", shell only):
// native title bar, sidebar, content area and the transport bar placeholder.
// Everything standard is a native control from the active style; every colour, size and gap comes from Theme.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Tinwave.Desktop

ApplicationWindow {
    id: root

    visible: true
    title: "Tinwave"
    // First-start size: the minimum plus room for the optional side panel; the last size is remembered later.
    width: Theme.windowMinWidth + Theme.sidePanelWidth
    height: Theme.windowMinHeight + Theme.sidePanelWidth
    minimumWidth: Theme.windowMinWidth
    minimumHeight: Theme.windowMinHeight
    color: Theme.windowFill

    // Native controls read palette roles; feed them from tokens so they agree with Tw* parts when the user
    // overrides the accent or the OS exposes none (design-system.md section 3.1, ADR 0007).
    palette.accent: Theme.accentFill
    palette.highlight: Theme.accentFill
    palette.highlightedText: Theme.onAccent

    // Developer/automation flag until Settings can own it: `--color-scheme=light|dark|system` switches the app's own
    // colour scheme in-process (the OS setting is never touched).
    Component.onCompleted: {
        const prefix = "--color-scheme="
        const arg = Qt.application.arguments.find(a => a.startsWith(prefix))
        if (arg !== undefined)
            Theme.setSchemeOverride(arg.substring(prefix.length))
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Sidebar {
            id: sidebar
            Layout.preferredWidth: Theme.sidebarWidth
            Layout.fillHeight: true
        }

        Rectangle {
            id: content
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.content

            // TwEmptyState-style: one sentence (the action arrives with the import ticket).
            Label {
                anchors.centerIn: parent
                text: "Your library is empty."
                font: Theme.fontSubtitle
                color: Theme.textMuted
            }
        }
    }

    // Transport bar placeholder: the real bar (TwTransportBar) arrives with the playback UI ticket.
    footer: Rectangle {
        implicitHeight: Theme.transportBarHeight
        color: Theme.window

        ToolSeparator {
            orientation: Qt.Horizontal
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            padding: 0
        }

        Label {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: Theme.l
            text: "Nothing playing"
            font: Theme.fontBody
            color: Theme.textMuted
        }
    }
}
