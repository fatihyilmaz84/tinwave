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
    // First-start size: the default, clamped once in Component.onCompleted (an assignment, not a binding, so a later
    // screen change never resizes the window back); the last size is remembered later.
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

    // OS facts the Theme follows, from the platform bridge (Rust `PlatformSettings`, ADR 0007): animation effects off,
    // a high-contrast theme, and whether Mica is applied behind the sidebar. Theme stays free of the bridge so its
    // contrast self-check can run standalone.
    Binding { target: Theme; property: "reducedMotion"; value: PlatformSettings.reducedMotion }
    Binding { target: Theme; property: "highContrast"; value: PlatformSettings.highContrast }
    Binding { target: Theme; property: "backdropActive"; value: PlatformSettings.backdropActive }

    // Native menus draw in the app's own scheme (the OS scheme while it is "system"), never light inside a dark app.
    Connections {
        target: Theme
        function onSchemeOverrideChanged() { PlatformSettings.setMenuScheme(Theme.schemeOverride) }
    }

    // Developer/automation flags until Settings can own them; they change the app in-process and never touch the OS:
    //   --color-scheme=light|dark|system   the app's own colour scheme
    //   --accent=#rrggbb                   the app's accent colour (pale/dark accents for the UI Definition of Done)
    //   --transparency=off|system          off forces the solid sidebar instead of Mica
    function switchValue(name) {
        const prefix = "--" + name + "="
        const arg = Qt.application.arguments.find(a => a.startsWith(prefix))
        return arg === undefined ? undefined : arg.substring(prefix.length)
    }

    Component.onCompleted: {
        width = Theme.firstStartWidth(Screen.desktopAvailableWidth)
        height = Theme.firstStartHeight(Screen.desktopAvailableHeight)

        const scheme = switchValue("color-scheme")
        if (scheme !== undefined)
            Theme.setSchemeOverride(scheme)
        const accent = switchValue("accent")
        if (accent !== undefined && !Theme.setAccentOverride(accent))
            console.warn("--accent expects #rrggbb, got '" + accent + "'")
        const transparency = switchValue("transparency")
        if (transparency === "off")
            PlatformSettings.setTransparencyAllowed(false)
        else if (transparency !== undefined && transparency !== "system")
            console.warn("--transparency expects off or system, got '" + transparency + "'")

        // Last, after the switches: watches the window for OS setting changes and applies the sidebar backdrop.
        PlatformSettings.attachWindow(root)
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
