// T-0011 prototype window: sidebar (Mica-capable) + a page of NATIVE controls driven only by Theme tokens.
// No control is restyled; the only things set on native controls are size/layout and palette roles from tokens.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Tinwave.Theme
import Tinwave.Spike

ApplicationWindow {
    id: root
    width: 1100
    height: 900
    visible: true
    title: "T-0011 native styles spike"
    color: Theme.windowFill

    readonly property var opts: PlatformInfo.options
    property string micaResult: "off"

    // Native controls read palette roles; we feed them from tokens so they agree with Tw* parts
    // even when the OS exposes no accent (Linux) or the user overrides it.
    palette.accent: opts.noAccentFix ? Theme.accent : Theme.accentFill
    palette.highlight: opts.noAccentFix ? Theme.accent : Theme.accentFill
    palette.highlightedText: Theme.onAccent

    Binding { target: Theme; property: "reducedMotion"; value: PlatformInfo.reducedMotion }
    Binding { target: Theme; property: "highContrast"; value: PlatformInfo.highContrast }
    Binding { target: Theme; property: "backdropActive"; value: root.micaResult === "ok" }

    Component.onCompleted: {
        if (opts.scheme && opts.scheme !== "system")
            Theme.setSchemeOverride(opts.scheme)
        if (opts.accent)
            Theme.accentOverride = opts.accent
        if (opts.mica && PlatformInfo.transparencyEffects)
            micaResult = PlatformInfo.applyMica(root, true)
        else if (opts.mica)
            micaResult = "skipped:transparency effects off in OS"
        PlatformInfo.log("INFO opts=" + JSON.stringify(opts))
        PlatformInfo.log("INFO style=" + opts.style + " scheme=" + (Theme.dark ? "dark" : "light")
            + " osAccent(palette.accent)=" + Theme.osAccent + " themeAccent=" + Theme.accent
            + " dwmRegistryAccent=" + PlatformInfo.dwmAccent)
        PlatformInfo.log("INFO mica=" + micaResult + " build=" + PlatformInfo.windowsBuild
            + " transparency=" + PlatformInfo.transparencyEffects)
        PlatformInfo.log("INFO qt.hints " + PlatformInfo.qtHints + " reducedMotion=" + PlatformInfo.reducedMotion
            + " highContrast=" + PlatformInfo.highContrast + "(" + PlatformInfo.highContrastScheme + ")")
        const f = Qt.application.font
        PlatformInfo.log("INFO appFont family=" + f.family + " pointSize=" + f.pointSize + " pixelSize=" + f.pixelSize
            + " -> Theme.base=" + Theme.base)
        PlatformInfo.log("INFO controlFonts button.px=" + probeButton.font.pixelSize + " button.pt=" + probeButton.font.pointSize
            + " family=" + probeButton.font.family + " | textfield.px=" + probeField.font.pixelSize + " | label.px=" + probeLabel.font.pixelSize
            + " | menuitem.px=" + ctxMenu.itemAt(0).font.pixelSize)
    }

    // ---- probes -----------------------------------------------------------------------------------------
    Timer {
        id: probeTimer
        interval: 2200
        running: !!root.opts.probe
        onTriggered: {
            const p = root.opts.probe
            PlatformInfo.log("PROBE start " + p + " style=" + root.opts.style)
            if (p === "menu") {
                PlatformInfo.log("PROBE menu popupType before=" + ctxMenu.popupType + " (Item=0 Window=1 Native=2)")
                PlatformInfo.scheduleProbe("menu", 900, true, "menu")
                ctxMenu.popup()   // blocking on Windows when native
                PlatformInfo.log("PROBE menu popup() returned; popupType after=" + ctxMenu.popupType)
                PlatformInfo.quitLater(300)
            } else if (p === "file") {
                PlatformInfo.scheduleProbe("file", 1800, true, "close")
                fileDlg.open()
                PlatformInfo.quitLater(3200)
            } else if (p === "folder") {
                PlatformInfo.scheduleProbe("folder", 1800, true, "close")
                folderDlg.open()
                PlatformInfo.quitLater(3200)
            } else if (p === "message") {
                PlatformInfo.scheduleProbe("message", 3500, true, "close")
                msgDlg.open()
                PlatformInfo.quitLater(5200)
            }
        }
    }
    Timer {  // live-plumbing test against our own window
        interval: 2000
        running: !!root.opts.motionTest
        onTriggered: {
            PlatformInfo.log("MOTIONTEST before: changeCount=" + PlatformInfo.changeCount)
            PlatformInfo.simulateSettingChange(root, "motion")
            PlatformInfo.simulateSettingChange(root, "contrast")
            PlatformInfo.simulateSettingChange(root, "theme")
            PlatformInfo.quitLater(600)
        }
    }
    Timer {  // live light/dark override (setColorScheme) round trip; logs what Theme and native palettes saw
        id: cycleTimer
        property int step: 0
        readonly property var seq: ["light", "dark", "system", "dark", "system"]
        interval: 800
        running: !!root.opts.schemeCycle
        repeat: true
        function report(tag) {
            PlatformInfo.log("CYCLE " + tag + ": colorScheme=" + Application.styleHints.colorScheme + " Theme.dark=" + Theme.dark
                + " Theme.window=" + Theme.window + " osAccent=" + Theme.osAccent + " accentText=" + Theme.accentText
                + " | native palette: window=" + probeButton.palette.window + " button=" + probeButton.palette.button
                + " accent=" + probeButton.palette.accent + " text=" + probeButton.palette.buttonText)
        }
        onTriggered: {
            if (step > 0) report("after '" + seq[step - 1] + "'")
            if (step < seq.length) {
                Theme.setSchemeOverride(seq[step])
                step++
            } else {
                running = false
                PlatformInfo.quitLater(100)
            }
        }
    }
    Connections {
        target: PlatformInfo
        function onChanged() {
            PlatformInfo.log("MOTIONTEST Theme saw reducedMotion=" + Theme.reducedMotion + " highContrast="
                + Theme.highContrast + " durationFast=" + Theme.durationFast + " changeCount=" + PlatformInfo.changeCount)
        }
    }

    Button { id: probeButton; visible: false }
    TextField { id: probeField; visible: false }
    Label { id: probeLabel; visible: false }

    Menu {
        id: ctxMenu
        popupType: opts.noNative ? Popup.Item : Popup.Native
        MenuItem { text: "Play next"; onTriggered: PlatformInfo.log("menu: play next") }
        MenuItem { text: "Add to queue"; onTriggered: PlatformInfo.log("menu: add to queue") }
        MenuSeparator {}
        Menu {
            title: "Add to playlist"
            MenuItem { text: "Road trip" }
            MenuItem { text: "Focus" }
        }
        MenuItem { text: "Show in folder"; enabled: false }
        MenuItem { text: "Favorite"; checkable: true; checked: true }
    }

    Loader {
        active: !opts.noDialogs
        sourceComponent: Item {}
    }
    FileDialog {
        id: fileDlg
        title: "Add audio file"
        currentFolder: opts.dlgFolder ? opts.dlgFolder : ""
        nameFilters: ["Audio (*.flac *.mp3 *.m4a *.ogg *.opus *.wav)", "All files (*)"]
        onAccepted: PlatformInfo.log("file accepted " + selectedFile)
    }
    FolderDialog {
        id: folderDlg
        title: "Add music folder"
        onAccepted: PlatformInfo.log("folder accepted " + selectedFolder)
    }
    MessageDialog {
        id: msgDlg
        title: "Remove from library?"
        text: "Remove 'Demo Album' from the library?"
        informativeText: "Your files are not deleted."
        buttons: MessageDialog.Yes | MessageDialog.No
        onAccepted: PlatformInfo.log("message accepted")
    }

    // ---- layout -----------------------------------------------------------------------------------------
    RowLayout {
        anchors.fill: parent
        spacing: 0

        // Sidebar: solid token, or transparent over Mica.
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: Theme.sidebarWidth
            color: Theme.sidebarFill

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.m
                spacing: Theme.s

                Label { text: "Tinwave"; font: Theme.fontSubtitle; color: Theme.text }
                TextField {
                    Layout.fillWidth: true
                    placeholderText: "Search"
                    Accessible.name: "Search library"
                }
                Label { text: "Library"; font: Theme.fontOverline; color: Theme.textMuted; Layout.topMargin: Theme.s }
                Repeater {
                    model: ["Songs", "Albums", "Artists", "Genres"]
                    delegate: ItemDelegate {
                        required property string modelData
                        required property int index
                        Layout.fillWidth: true
                        text: modelData
                        highlighted: index === 1
                    }
                }
                Label { text: "Listen"; font: Theme.fontOverline; color: Theme.textMuted; Layout.topMargin: Theme.s }
                Repeater {
                    model: ["Smart Radio", "Insights"]
                    delegate: ItemDelegate { required property string modelData; Layout.fillWidth: true; text: modelData }
                }
                Item { Layout.fillHeight: true }
                Label {
                    text: "Backdrop: " + (Theme.backdropActive ? "Mica" : "solid")
                    font: Theme.fontCaption
                    color: Theme.textMuted
                }
            }
        }

        Rectangle { Layout.fillHeight: true; Layout.preferredWidth: 1; color: Theme.separator }

        // Content surface
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.content

            ScrollView {
                id: scroll
                anchors.fill: parent
                contentWidth: availableWidth
                clip: true

                ColumnLayout {
                    width: scroll.availableWidth
                    spacing: Theme.l

                    // padding wrapper
                    ColumnLayout {
                        Layout.margins: Theme.xl
                        Layout.fillWidth: true
                        spacing: Theme.l

                        RowLayout {
                            Layout.fillWidth: true
                            Label { text: "Native controls"; font: Theme.fontTitle; color: Theme.text; Layout.fillWidth: true }
                            Label {
                                text: opts.style + " / " + (Theme.dark ? "dark" : "light")
                                font: Theme.fontCaption
                                color: Theme.textMuted
                            }
                        }

                        // --- live OS state (for the PO runbook: change a Windows setting, watch this line) ----------
                        Label { text: "Live OS state"; font: Theme.fontOverline; color: Theme.textMuted }
                        Label {
                            Layout.fillWidth: true
                            wrapMode: Text.Wrap
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            text: "colorScheme=" + (Theme.dark ? "dark" : "light") + "  palette.accent=" + Theme.osAccent
                                + "  reducedMotion=" + PlatformInfo.reducedMotion + "  highContrast=" + PlatformInfo.highContrast
                                + (PlatformInfo.highContrastScheme ? " (" + PlatformInfo.highContrastScheme + ")" : "")
                                + "  transparencyEffects=" + PlatformInfo.transparencyEffects
                                + "  Qt hints: " + PlatformInfo.qtHints
                                + "  appFont=" + Qt.application.font.pointSize + "pt  durationFast=" + Theme.durationFast
                                + "  OS setting events seen=" + PlatformInfo.changeCount
                        }

                        // --- accent tokens -------------------------------------------------------------
                        Label { text: "Accent tokens"; font: Theme.fontOverline; color: Theme.textMuted }
                        RowLayout {
                            spacing: Theme.m
                            Repeater {
                                model: [
                                    { name: "accent", fill: Theme.accent, fg: Theme.onAccent },
                                    { name: "accentFill", fill: Theme.accentFill, fg: Theme.onAccent },
                                    { name: "selected", fill: Theme.selected, fg: Theme.text },
                                    { name: "raised", fill: Theme.raised, fg: Theme.text }
                                ]
                                delegate: Rectangle {
                                    required property var modelData
                                    implicitWidth: 120; implicitHeight: 40
                                    radius: Theme.radiusControl
                                    color: modelData.fill
                                    border.width: 1
                                    border.color: Theme.separator
                                    Label { anchors.centerIn: parent; text: parent.modelData.name; color: parent.modelData.fg }
                                }
                            }
                            ColumnLayout {
                                spacing: 0
                                Label {
                                    text: "Accent as text (accentText)"
                                    font: Theme.fontBodyStrong
                                    color: Theme.accentText
                                }
                                Label {
                                    text: "raw accent " + Theme.accent.toString() + " " + Theme.contrast(Theme.accent, Theme.content).toFixed(2)
                                        + ":1  ->  accentText " + Theme.accentText.toString() + " "
                                        + Theme.contrast(Theme.accentText, Theme.content).toFixed(2) + ":1"
                                    font: Theme.fontCaption
                                    color: Theme.textMuted
                                }
                            }
                        }

                        // --- buttons -------------------------------------------------------------------
                        Label { text: "Buttons"; font: Theme.fontOverline; color: Theme.textMuted }
                        RowLayout {
                            spacing: Theme.s
                            Button { text: "Add folder…"; onClicked: folderDlg.open() }
                            Button { text: "Primary"; highlighted: true }
                            Button { text: "Disabled"; enabled: false }
                            ToolButton { text: "Tool" ; Accessible.name: "Tool button" }
                            Button { text: "Menu…"; onClicked: ctxMenu.popup() }
                            Button { text: "Choose file…"; onClicked: fileDlg.open() }
                            Button { text: "Confirm…"; onClicked: msgDlg.open() }
                        }

                        // --- inputs --------------------------------------------------------------------
                        Label { text: "Inputs"; font: Theme.fontOverline; color: Theme.textMuted }
                        RowLayout {
                            spacing: Theme.m
                            TextField { placeholderText: "Text field"; text: "Sample text"; Layout.preferredWidth: 200 }
                            ComboBox { model: ["Newest first", "Title", "Artist", "Plays"]; Layout.preferredWidth: 160 }
                            SpinBox { value: 5 }
                        }
                        RowLayout {
                            spacing: Theme.l
                            Switch { text: "Follow system theme"; checked: true }
                            CheckBox { text: "Remember window size"; checked: true }
                            RadioButton { text: "Comfortable"; checked: true }
                            RadioButton { text: "Compact" }
                        }
                        RowLayout {
                            spacing: Theme.l
                            Slider { value: 0.4; Layout.preferredWidth: 220 }
                            ProgressBar { value: 0.6; Layout.preferredWidth: 160 }
                            ProgressBar { indeterminate: true; Layout.preferredWidth: 120 }
                        }
                        TabBar {
                            TabButton { text: "Queue" }
                            TabButton { text: "Lyrics" }
                            TabButton { text: "Info" }
                        }

                        // --- Tw-like rows built from tokens only ---------------------------------------
                        Label { text: "Track rows (token-only, Tw-style)"; font: Theme.fontOverline; color: Theme.textMuted }
                        Repeater {
                            model: [
                                { t: "Weightless", a: "Marconi Union", d: "8:08", state: "default" },
                                { t: "Teardrop", a: "Massive Attack", d: "5:29", state: "selected" },
                                { t: "Nuvole Bianche", a: "Ludovico Einaudi", d: "5:57", state: "hover" }
                            ]
                            delegate: Rectangle {
                                id: rowRect
                                required property var modelData
                                Layout.fillWidth: true
                                implicitHeight: Theme.rowHeight
                                radius: Theme.radiusControl
                                color: modelData.state === "selected" ? Theme.selected : "transparent"
                                Rectangle {
                                    anchors.fill: parent
                                    radius: parent.radius
                                    color: rowRect.modelData.state === "hover" ? Theme.hover : "transparent"
                                }
                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: Theme.m
                                    anchors.rightMargin: Theme.m
                                    spacing: Theme.m
                                    Rectangle {
                                        implicitWidth: 28; implicitHeight: 28
                                        radius: Theme.radiusArtSmall
                                        color: Theme.raised
                                        border.width: 1; border.color: Theme.separator
                                    }
                                    Label { text: rowRect.modelData.t; font: Theme.fontBodyStrong; color: Theme.text; Layout.preferredWidth: 160 }
                                    Label { text: rowRect.modelData.a; color: Theme.textMuted; Layout.fillWidth: true }
                                    Label { text: "Local"; font: Theme.fontCaption; color: Theme.srcDownload }
                                    Label {
                                        text: rowRect.modelData.d
                                        font.pixelSize: Theme.sizeCaption
                                        font.features: Theme.tabular
                                        color: Theme.textFaint
                                    }
                                }
                                TapHandler { acceptedButtons: Qt.RightButton; onTapped: ctxMenu.popup() }
                            }
                        }

                        // --- type scale ----------------------------------------------------------------
                        Label { text: "Type scale (base " + Theme.base + " px)"; font: Theme.fontOverline; color: Theme.textMuted }
                        Label { text: "Display"; font: Theme.fontDisplay; color: Theme.text }
                        Label { text: "Title / Subtitle"; font: Theme.fontSubtitle; color: Theme.text }
                        Label { text: "Body, body strong, caption 0:42 / 3:58"; font: Theme.fontBody; color: Theme.text }
                        Label {
                            text: "Warning budget 82 %"
                            font: Theme.fontBodyStrong
                            color: Theme.warning
                        }
                        Label { text: "Danger: wipe all data"; font: Theme.fontBodyStrong; color: Theme.danger }
                        Label { text: "Server: Home NAS"; font: Theme.fontBodyStrong; color: Theme.srcServer }
                    }
                }
            }
        }
    }
}
