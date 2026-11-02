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
    // First-start size; the last size is remembered later.
    width: Theme.windowDefaultWidth
    height: Theme.windowDefaultHeight
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

        // Content surface: rounded top-left corner against the sidebar, outlined by a token hairline on its top and left
        // edges (canvas "Library - Windows"). The sidebar material continues behind the rounded corner.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Rectangle {
                anchors.fill: parent
                color: Theme.sidebarFill
            }

            Rectangle {
                anchors.fill: parent
                topLeftRadius: Theme.radiusContent
                color: Theme.separator
            }

            Rectangle {
                id: content
                anchors.fill: parent
                anchors.topMargin: Theme.hairline
                anchors.leftMargin: Theme.hairline
                topLeftRadius: Theme.radiusContent - Theme.hairline
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
    }

    // Transport bar placeholder: the real bar (TwTransportBar) arrives with the playback UI ticket.
    footer: Rectangle {
        implicitHeight: Theme.transportBarHeight
        color: Theme.window

        // Hairline in the `separator` token (the style's ToolSeparator draws its own inset colour).
        Rectangle {
            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            height: Theme.hairline
            color: Theme.separator
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
