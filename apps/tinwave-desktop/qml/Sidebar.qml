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
                text: "Menu"
                Accessible.name: "App menu"
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
                ListElement { group: "Library"; key: "songs"; label: "Songs" }
                ListElement { group: "Library"; key: "albums"; label: "Albums" }
                ListElement { group: "Library"; key: "artists"; label: "Artists" }
                ListElement { group: "Library"; key: "genres"; label: "Genres" }
                ListElement { group: "Listen"; key: "radio"; label: "Smart Radio" }
                ListElement { group: "Listen"; key: "insights"; label: "Insights" }
                ListElement { group: "Sources"; key: "this-computer"; label: "This computer" }
                ListElement { group: "Sources"; key: "downloads"; label: "Downloads" }
                ListElement { group: "Manage"; key: "import"; label: "Import & organize" }
                ListElement { group: "Manage"; key: "ripping"; label: "CD ripping" }
                ListElement { group: "Manage"; key: "signal-center"; label: "Signal Center" }
                ListElement { group: "Manage"; key: "settings"; label: "Settings" }
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

                width: ListView.view.width
                height: Theme.sidebarRowHeight
                // `&` marks a mnemonic in button text; show it literally.
                text: label.replace(/&/g, "&&")
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
