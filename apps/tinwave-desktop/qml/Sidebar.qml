// Sidebar (design-system.md section 4): app menu, search, and the four navigation groups as native ItemDelegates.
// Pure presentation: navigation targets arrive with the screen tickets, so `currentKey` is only reported upward.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Tinwave.Desktop

Rectangle {
    id: root

    // Key of the selected destination (stable id, never the display text).
    property string currentKey: "songs"

    color: Theme.sidebarFill

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.s
        spacing: Theme.xs

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.s

            ToolButton {
                id: menuButton
                // Icon only: the native icon slot, tinted from tokens; the name is for assistive technology and the tooltip.
                display: AbstractButton.IconOnly
                icon.source: TwIcons.source("menu")
                icon.width: Theme.iconSize
                icon.height: Theme.iconSize
                icon.color: Theme.text
                Accessible.name: "App menu"
                ToolTip.text: "App menu"
                ToolTip.visible: hovered
                onClicked: appMenu.popup(menuButton, 0, menuButton.height)

                Menu {
                    id: appMenu
                    // popupType is pending ADR 0007 decision 1 (Popup.Native plus the dark-mode opt-in, falling back
                    // to Popup.Window); it is wired together with the platform bridge in T-0036.

                    MenuItem {
                        text: "Tinwave " + AppInfo.version
                        enabled: false
                    }
                    MenuSeparator {}
                    MenuItem {
                        text: "Quit"
                        onTriggered: Qt.quit()
                    }
                }
            }

            TextField {
                id: search
                Layout.fillWidth: true
                placeholderText: "Search"
                Accessible.name: "Search"
            }
        }

        ListView {
            id: nav
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar {}
            currentIndex: 0
            keyNavigationEnabled: true

            model: ListModel {
                ListElement { group: "Library"; key: "songs"; label: "Songs"; iconName: "eighth-notes" }
                ListElement { group: "Library"; key: "albums"; label: "Albums"; iconName: "disc" }
                ListElement { group: "Library"; key: "artists"; label: "Artists"; iconName: "user-round" }
                ListElement { group: "Library"; key: "genres"; label: "Genres"; iconName: "layout-grid" }
                ListElement { group: "Listen"; key: "radio"; label: "Smart Radio"; iconName: "radio" }
                ListElement { group: "Listen"; key: "insights"; label: "Insights"; iconName: "chart-column" }
                ListElement { group: "Sources"; key: "this-computer"; label: "This computer"; iconName: "folder" }
                ListElement { group: "Sources"; key: "downloads"; label: "Downloads"; iconName: "arrow-down-to-line" }
                ListElement { group: "Manage"; key: "import"; label: "Import & organize"; iconName: "upload" }
                ListElement { group: "Manage"; key: "ripping"; label: "CD ripping"; iconName: "disc-3" }
                ListElement { group: "Manage"; key: "signal-center"; label: "Signal Center"; iconName: "activity" }
                ListElement { group: "Manage"; key: "settings"; label: "Settings"; iconName: "sliders-horizontal" }
            }

            section.property: "group"
            section.criteria: ViewSection.FullString
            section.delegate: Label {
                required property string section
                width: ListView.view.width
                topPadding: Theme.m
                bottomPadding: Theme.xs
                leftPadding: Theme.m
                text: section
                font: Theme.fontOverline
                color: Theme.textMuted
            }

            delegate: ItemDelegate {
                required property int index
                required property string key
                required property string label
                required property string iconName

                width: ListView.view.width
                height: Theme.sidebarRowHeight
                // `&` marks a mnemonic in button text; show it literally.
                text: label.replace(/&/g, "&&")
                // Native icon slot, tinted from tokens: `text` for every row (canvas "Library - Windows"), gap `Theme.m`.
                icon.source: TwIcons.source(iconName)
                icon.width: Theme.iconSize
                icon.height: Theme.iconSize
                icon.color: Theme.text
                spacing: Theme.m
                // One tab stop for the whole list (the current item); Up/Down move the selection.
                focusPolicy: ListView.isCurrentItem ? Qt.StrongFocus : Qt.ClickFocus
                highlighted: ListView.isCurrentItem
                Accessible.name: label
                onClicked: nav.currentIndex = index
            }

            onCurrentIndexChanged: root.currentKey = currentItem ? currentItem.key : ""
        }
    }
}
