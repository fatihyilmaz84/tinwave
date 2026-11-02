import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// TwTrackRow prototype: native ItemDelegate base (FluentWinUI3), our content only.
ItemDelegate {
    id: root

    required property int index
    required property string title
    required property string artist
    required property string album
    required property string duration
    required property string artKey
    required property bool loaded

    signal hoverChanged(int index, bool entered)

    width: ListView.view ? ListView.view.width : implicitWidth
    implicitHeight: Theme.rowHeight
    highlighted: ListView.isCurrentItem

    HoverHandler {
        onHoveredChanged: root.hoverChanged(root.index, hovered)
    }

    contentItem: RowLayout {
        spacing: Theme.spaceM

        TwArtwork {
            artKey: root.artKey
            loaded: root.loaded
        }
        Label {
            Layout.fillWidth: true
            text: root.loaded ? root.title : ""
            elide: Text.ElideRight
        }
        Label {
            Layout.preferredWidth: Theme.artistColumn
            text: root.artist
            elide: Text.ElideRight
            color: Theme.textMuted
            font.pixelSize: Theme.fontCaption
        }
        Label {
            Layout.preferredWidth: Theme.durationColumn
            horizontalAlignment: Text.AlignRight
            text: root.duration
            color: Theme.textMuted
            font.pixelSize: Theme.fontCaption
            font.features: { "tnum": 1 }
        }
    }
}
