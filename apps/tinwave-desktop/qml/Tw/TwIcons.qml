// Where the vendored Lucide icons live (design-system.md section 3.4). Native controls take an icon through their own
// slot (`icon.source: TwIcons.source(name)`); TwIcon is for places without such a slot. Icon names are the Lucide names
// and are always written as string literals so the build check (`check_icon_usage` in build.rs) can match them against
// the files in icons/.
pragma Singleton
import QtQuick

QtObject {
    // The icons are resources of this QML module (build.rs): resolved relative to this file, whatever the module prefix.
    function source(name) {
        return Qt.resolvedUrl("../../icons/" + name + ".svg")
    }
}
