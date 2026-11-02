// Contrast self-test: runs Theme.resolve() for many accents in both schemes and prints WCAG ratios.
// A row FAILS when accentText < 4.5 on any surface, or onAccent < 4.5 on the accent.
import QtQuick
import Tinwave.Theme
import Tinwave.Spike

QtObject {
    id: root

    function f(x) { return x.toFixed(2) }
    function hex(c) { return Qt.rgba(c.r, c.g, c.b, 1).toString().toUpperCase() }

    Component.onCompleted: {
        const log = PlatformInfo.log
        let fails = 0
        // Windows default blue, Tin Teal, Windows palette picks (yellow, pink, green, red, orange, grey), pale/dark extremes.
        const accents = ["#0067C0", "#0B6E78", "#FFB900", "#E3008C", "#10893E", "#D13438", "#FF8C00", "#767676",
                         "#00B7C3", "#FFFFFF", "#000000", "#F0E68C", "#7A7574", "#5CC6CF", "#99FF99"]
        // a) every fixed text/surface pair from design-system.md 3.1
        for (const dark of [false, true]) {
            const t = Theme.resolve(dark, Qt.color("#0B6E78"), false)
            const surf = { window: t.window, sidebar: t.sidebar, content: t.content, raised: t.raised }
            const texts = ["text", "textMuted", "textFaint", "srcServer", "srcDownload", "warning", "danger"]
            for (const sname in surf) {
                let line = (dark ? "dark " : "light") + " on " + sname.padEnd(7) + ":"
                for (const tn of texts) {
                    const c = Theme.contrast(Qt.color(t[tn]), Qt.color(surf[sname]))
                    line += " " + tn + "=" + f(c) + (c < 4.5 ? "!" : "")
                    if (c < 4.5) fails++
                }
                log("FIXED " + line)
            }
            // text on the selected tint (Tin Teal)
            for (const tn of ["text", "textMuted", "textFaint"]) {
                const c = Theme.contrast(Qt.color(t[tn]), Qt.color(t.selected))
                log("FIXED " + (dark ? "dark " : "light") + " " + tn + " on selected(" + hex(Qt.color(t.selected)) + ")=" + f(c) + (c < 4.5 ? "!" : ""))
                if (c < 4.5) fails++
            }
            log("FIXED " + (dark ? "dark " : "light") + " onAccent(default) on TinTeal=" + f(Theme.contrast(Qt.color(t.onAccent), Qt.color("#0B6E78"))))
        }
        // b) selected tint values vs the spec hex
        for (const [dark, accent, specHex] of [[false, "#0B6E78", "#E6F1F2"], [true, "#5CC6CF", "#1B2B2D"]]) {
            const t = Theme.resolve(dark, Qt.color(accent), false)
            log("SELECTED " + (dark ? "dark" : "light") + " computed " + hex(Qt.color(t.selected)) + " spec " + specHex)
        }
        // c) arbitrary accents
        for (const dark of [false, true]) {
            for (const a of accents) {
                const acc = Qt.color(a)
                const t = Theme.resolve(dark, acc, false)
                const surfaces = [t.window, t.sidebar, t.content, t.raised, t.selected].map(c => Qt.color(c))
                let rawMin = 1000, atMin = 1000
                for (const s of surfaces) {
                    rawMin = Math.min(rawMin, Theme.contrast(acc, s))
                    atMin = Math.min(atMin, Theme.contrast(Qt.color(t.accentText), s))
                }
                const oa = Theme.contrast(Qt.color(t.onAccent), acc)
                // FluentWinUI3 label on accented controls: white (light) / black (dark), hard-coded by the style
                const fixedLabel = Qt.color(dark ? "#000000" : "#FFFFFF")
                const rawFill = Theme.contrast(acc, fixedLabel)
                const fillC = Theme.contrast(Qt.color(t.accentFill), fixedLabel)
                const ok = atMin >= 4.5 && oa >= 4.5 && fillC >= 4.5
                if (!ok) fails++
                log("ACCENT " + (dark ? "dark " : "light") + " " + a + " raw(min over surfaces)=" + f(rawMin)
                    + " -> accentText " + hex(Qt.color(t.accentText)) + " min=" + f(atMin)
                    + " | onAccent " + hex(Qt.color(t.onAccent)) + " on accent=" + f(oa)
                    + " | native fill: raw " + f(rawFill) + " -> accentFill " + hex(Qt.color(t.accentFill)) + " " + f(fillC)
                    + " " + (ok ? "ok" : "FAIL"))
            }
        }
        // d) the real OS accent as Qt reports it (palette.accent), both schemes
        for (const dark of [false, true]) {
            const t = Theme.resolve(dark, Theme.osAccent, false)
            log("OSACCENT " + (dark ? "dark " : "light") + " palette.accent=" + hex(Theme.osAccent) + " accentText=" + hex(Qt.color(t.accentText))
                + " c(accentText,content)=" + f(Theme.contrast(Qt.color(t.accentText), Qt.color(t.content))))
        }
        // e) font probes: Qt.font({...}) round trip and features support
        const probe = Theme.fontOverline
        log("FONT overline pixelSize=" + probe.pixelSize + " capitalization=" + probe.capitalization + " letterSpacing=" + probe.letterSpacing + " family=" + probe.family)
        log("FONT title weight=" + Theme.fontTitle.weight + " family=" + Theme.fontTitle.family)
        log("FONT base=" + Theme.base + " appFont.pixelSize=" + Qt.application.font.pixelSize + " appFont.pointSize=" + Qt.application.font.pointSize)
        // e2) high-contrast branch must define exactly the same token keys as the normal branch (no undefined tokens)
        const normalKeys = Object.keys(Theme.resolve(false, Qt.color("#0B6E78"), false)).sort().join(",")
        const hcKeys = Object.keys(Theme.resolve(false, Qt.color("#0B6E78"), true)).sort().join(",")
        log("HCKEYS parity=" + (normalKeys === hcKeys) + (normalKeys === hcKeys ? "" : " normal=" + normalKeys + " hc=" + hcKeys))
        if (normalKeys !== hcKeys) fails++
        // f) API surface the ticket/design system names
        log("API Application.styleHints.colorScheme=" + Application.styleHints.colorScheme
            + " | Qt.styleHints.colorScheme=" + (Qt.styleHints ? Qt.styleHints.colorScheme : "n/a")
            + " | styleHints.accentColor=" + (Application.styleHints.accentColor === undefined ? "undefined (no such property)" : Application.styleHints.accentColor)
            + " | styleHints.accessibility.contrastPreference=" + Application.styleHints.accessibility.contrastPreference
            + " | SystemPalette.accent=" + hex(Theme.osAccent) + " | Qt.application.font.pixelSize=" + Qt.application.font.pixelSize)
        log("SELFTEST fails=" + fails)
        PlatformInfo.quitLater(0)
    }
}
