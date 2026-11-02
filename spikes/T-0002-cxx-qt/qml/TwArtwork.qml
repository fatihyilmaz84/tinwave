import QtQuick

// Tw component: flat "raised" placeholder, async provider thumbnail, size-specific request.
Item {
    id: root
    property string artKey
    property bool loaded: true
    property int size: Theme.artSmall

    implicitWidth: size
    implicitHeight: size

    Rectangle {
        anchors.fill: parent
        radius: Theme.radiusArtSmall
        color: Theme.raised
    }
    Image {
        anchors.fill: parent
        asynchronous: true
        cache: false            // the Rust LRU cache owns thumbnails; keep QML's pixmap cache out of RSS
        fillMode: Image.PreserveAspectCrop
        source: root.loaded && root.artKey !== "" ? "image://art/" + root.artKey : ""
        sourceSize: Qt.size(root.size, root.size)   // logical px: Qt multiplies by devicePixelRatio itself
    }
}
