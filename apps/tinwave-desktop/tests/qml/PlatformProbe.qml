// Test-only root (loaded through TINWAVE_MAIN_QML in debug builds by tests/platform_qml.rs): instantiates the real
// window shell, which binds the platform bridge into Theme and applies the developer switches, then prints what the
// test checks as `PROBE name=value` lines and quits.
import QtQuick
import Tinwave.Desktop

QtObject {
    id: probe

    property Main shell: Main {}

    function say(name, value) { console.log("PROBE " + name + "=" + value) }

    property Timer finish: Timer {
        interval: 50
        running: true
        onTriggered: {
            probe.say("settingsStatus", PlatformSettings.settingsStatus)
            probe.say("nativeMenus", PlatformSettings.nativeMenus)
            probe.say("menuStatus", PlatformSettings.menuStatus)
            probe.say("backdropActive", PlatformSettings.backdropActive)
            probe.say("backdropStatus", PlatformSettings.backdropStatus)
            probe.say("transparencyEffects", PlatformSettings.transparencyEffects)
            // The Theme follows the bridge through the bindings in Main.qml.
            probe.say("themeReducedMotionFollows", Theme.reducedMotion === PlatformSettings.reducedMotion)
            probe.say("themeHighContrastFollows", Theme.highContrast === PlatformSettings.highContrast)
            probe.say("themeBackdropFollows", Theme.backdropActive === PlatformSettings.backdropActive)
            probe.say("durationFast", Theme.durationFast)
            probe.say("accent", Theme.accent)
            probe.say("accentIsOverride", Theme.accentOverride !== "")
            probe.say("dark", Theme.dark)
            // The accent setter refuses anything but #rrggbb and "" resets.
            probe.say("bogusAccentAccepted", Theme.setAccentOverride("teal"))
            probe.say("shortAccentAccepted", Theme.setAccentOverride("#fff"))
            probe.say("alphaAccentAccepted", Theme.setAccentOverride("#80ff0000"))
            probe.say("accentAfterRefusals", Theme.accent)
            probe.say("goodAccentAccepted", Theme.setAccentOverride("#A1B2C3"))
            probe.say("accentAfterGood", Theme.accent)
            probe.say("resetAccepted", Theme.setAccentOverride(""))
            probe.say("accentOverrideAfterReset", Theme.accentOverride === "")
            Qt.quit()
        }
    }
}
