// Design tokens (design-system.md section 3). Ported unchanged from the T-0011 prototype; the token fixes it found (T-0035) land separately.
//
// Shape: ONE pure function `resolve(dark, accent, hc)` maps (scheme, accent) -> every token. The
// live properties bind to its result, and the same function can be called for arbitrary accents (contrast checks).
// Inputs that come from Qt itself (no bridge needed on Qt >= 6.8):
//   - Application.styleHints.colorScheme   (light/dark, writable: user override == setColorScheme)
//   - SystemPalette.accent / .highlight ... (QPalette::Accent, follows the OS accent on Windows/macOS)
//   - Qt.application.font                   (system font)
// Inputs the bridge must supply (platform bridge, T-0036; Qt 6.10 adds Application.styleHints.accessibility, 6.12 motionPreference):
//   - reducedMotion, highContrast, backdropActive
pragma Singleton
import QtQuick
import QtQuick.Controls

QtObject {
    id: theme

    // ------------------------------------------------------------------ inputs
    // Bridge-supplied (see PlatformInfo in the host; in the app: Rust PlatformSettings view model).
    property bool reducedMotion: false      // OS "animation effects" off
    property bool highContrast: false       // OS high-contrast theme active
    property bool backdropActive: false     // Mica / vibrancy currently applied to the sidebar
    // Whether the OS exposes an accent colour at all. Windows and macOS always do; on Linux only some
    // desktops do (the bridge sets this from the XDG portal); until then assume not and use Tin Teal.
    property bool osAccentAvailable: Qt.platform.os !== "linux"

    // User overrides (Settings). "" / "system" == follow the OS.
    property string accentOverride: ""      // "#rrggbb" or "" (follow OS)
    property string schemeOverride: "system" // "system" | "light" | "dark"

    function setSchemeOverride(mode) {
        schemeOverride = mode
        // QStyleHints::setColorScheme(): in Qt >= 6.8 this also re-resolves the application palette,
        // so native controls and the window title bar follow. Unknown == unset (follow the OS).
        Application.styleHints.colorScheme = mode === "dark" ? Qt.Dark : mode === "light" ? Qt.Light : Qt.Unknown
    }

    readonly property SystemPalette sys: SystemPalette { colorGroup: SystemPalette.Active }

    readonly property bool dark: Application.styleHints.colorScheme === Qt.Dark
    readonly property color osAccent: sys.accent
    readonly property bool followsOsAccent: accentOverride === "" && osAccentAvailable

    // ------------------------------------------------------------------ colour maths (WCAG 2.x)
    function _lin(c) { return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4) }
    function luminance(c) { return 0.2126 * _lin(c.r) + 0.7152 * _lin(c.g) + 0.0722 * _lin(c.b) }
    function contrast(a, b) {
        const la = luminance(a), lb = luminance(b)
        return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
    }
    function _q(x) { return Math.round(x * 255) / 255 }   // 8-bit quantise: the ratio must hold for the stored colour
    function mix(a, b, t) { return Qt.rgba(_q(a.r + (b.r - a.r) * t), _q(a.g + (b.g - a.g) * t), _q(a.b + (b.b - a.b) * t), 1) }
    // Composite a translucent colour over an opaque surface.
    function over(fg, alpha, bg) { return mix(bg, fg, alpha) }
    function _minContrast(fg, surfaces) {
        let m = 1000
        for (const s of surfaces) m = Math.min(m, contrast(fg, s))
        return m
    }
    // Smallest shift of `fg` toward black (light scheme) / white (dark scheme) such that it reaches `target`
    // against EVERY surface it may sit on. Hue is preserved (straight sRGB mix), so the OS accent stays recognisable.
    function ensureContrast(fg, surfaces, towardWhite, target) {
        if (_minContrast(fg, surfaces) >= target) return fg
        const end = towardWhite ? Qt.rgba(1, 1, 1, 1) : Qt.rgba(0, 0, 0, 1)
        let lo = 0, hi = 1
        for (let i = 0; i < 16; ++i) {
            const mid = (lo + hi) / 2
            if (_minContrast(mix(fg, end, mid), surfaces) >= target) hi = mid; else lo = mid
        }
        return mix(fg, end, hi)
    }
    // Best foreground for text ON an accent fill: the scheme default if it passes, else white, else ink.
    function onAccentFor(accent, preferred) {
        const white = Qt.rgba(1, 1, 1, 1), ink = Qt.color("#0F2F33")
        const cands = [preferred, white, ink, Qt.rgba(0, 0, 0, 1)]
        let best = cands[0], bestC = 0
        for (const c of cands) {
            const v = contrast(c, accent)
            if (v >= 4.5) return c
            if (v > bestC) { bestC = v; best = c }
        }
        return best
    }

    // ------------------------------------------------------------------ token resolution
    readonly property color tinTeal: "#0B6E78"
    readonly property color tinTealDark: "#5CC6CF"

    // dark/light fixed values from design-system.md section 3.1 (CSS #RRGGBBAA there -> rgba() here:
    // QML parses "#RRGGBBAA" as #AARRGGBB, so hex-with-alpha must NEVER be copied verbatim).
    function resolve(isDark, accent, hc) {
        const t = {}
        if (hc) {
            // Windows high contrast: system palette only, no tints, no hue-only meaning.
            t.window = sys.window; t.sidebar = sys.window; t.content = sys.base; t.raised = sys.window
            t.text = sys.windowText; t.textMuted = sys.windowText; t.textFaint = sys.windowText
            t.accent = sys.highlight; t.onAccent = sys.highlightedText; t.accentText = sys.link; t.accentFill = sys.highlight
            t.selected = sys.highlight; t.hover = Qt.rgba(0, 0, 0, 0); t.separator = sys.windowText
            t.srcServer = sys.windowText; t.srcDownload = sys.windowText
            t.warning = sys.windowText; t.danger = sys.windowText
            return t
        }
        const L = !isDark
        t.window = L ? "#F3F3F5" : "#161618"
        t.sidebar = L ? "#ECECEF" : "#1D1D20"
        t.content = L ? "#FFFFFF" : "#161618"
        t.raised = L ? "#FFFFFF" : "#202023"
        t.hover = L ? Qt.rgba(0, 0, 0, 0x0A / 255) : Qt.rgba(1, 1, 1, 0x0F / 255)
        t.separator = L ? Qt.rgba(0, 0, 0, 0x1A / 255) : Qt.rgba(1, 1, 1, 0x14 / 255)
        t.text = L ? "#1B1B1F" : "#F2F2F4"
        t.textMuted = L ? "#5D5D66" : "#A6A6AF"
        t.textFaint = L ? "#6E6E77" : "#8C8C95"
        t.srcServer = L ? "#1F5FA8" : "#7BB0EE"
        t.srcDownload = L ? "#2E7D46" : "#76C48E"
        t.warning = L ? "#9A5B00" : "#E0A453"
        t.danger = L ? "#B42318" : "#F2877D"
        t.accent = accent
        // selected = accent over the content surface. The spec's hex values correspond to ~10 % (light) and
        // ~12 % (dark), not the stated 12 % / 18 % -- see ADR-draft; kept as named constants below.
        t.selected = over(accent, L ? selectedAlphaLight : selectedAlphaDark, Qt.color(t.content))
        t.onAccent = onAccentFor(accent, L ? "#FFFFFF" : "#0F2F33")
        // accent used as TEXT must reach 4.5:1 on every surface text can sit on.
        const surfaces = [t.window, t.sidebar, t.content, t.raised, t.selected].map(c => Qt.color(c))
        t.accentText = ensureContrast(Qt.color(accent), surfaces, !L, 4.5)
        // accentFill: accent used as the FILL of NATIVE accented controls (Button.highlighted, Switch, CheckBox).
        // FluentWinUI3 hard-codes the label colour on those fills (white in light, black in dark -- it ignores
        // palette.highlightedText), so the fill itself must give 4.5:1 against that fixed colour. Feed this to
        // `palette.accent` on the window, not `accent`.
        const fixedLabel = Qt.color(L ? "#FFFFFF" : "#000000")
        t.accentFill = ensureContrast(Qt.color(accent), [fixedLabel], !L, 4.5)
        return t
    }
    readonly property real selectedAlphaLight: 0.10
    readonly property real selectedAlphaDark: 0.12

    readonly property color accent: accentOverride !== "" ? Qt.color(accentOverride)
                                    : osAccentAvailable ? osAccent : (dark ? tinTealDark : tinTeal)
    readonly property var _t: resolve(dark, accent, highContrast)

    readonly property color window: _t.window
    readonly property color sidebar: _t.sidebar
    readonly property color content: _t.content
    readonly property color raised: _t.raised
    readonly property color hover: _t.hover
    readonly property color selected: _t.selected
    readonly property color separator: _t.separator
    readonly property color text: _t.text
    readonly property color textMuted: _t.textMuted
    readonly property color textFaint: _t.textFaint
    readonly property color onAccent: _t.onAccent
    readonly property color accentText: _t.accentText
    readonly property color accentFill: _t.accentFill
    readonly property color srcServer: _t.srcServer
    readonly property color srcDownload: _t.srcDownload
    readonly property color warning: _t.warning
    readonly property color danger: _t.danger

    // Fill helpers for the window effect trait: when Mica/vibrancy is on, the sidebar (and window) must be
    // transparent so the DWM material shows through; the solid token is the fallback.
    readonly property color transparent: Qt.rgba(0, 0, 0, 0)
    readonly property color sidebarFill: backdropActive && !highContrast ? transparent : sidebar
    readonly property color windowFill: backdropActive && !highContrast ? transparent : window

    // ------------------------------------------------------------------ type (3.2)
    // `base` must be the size the NATIVE controls draw with, or Tw* text looks smaller/larger than a native Button next to it.
    // Measured on Windows/Qt 6.12: Qt.application.font = Segoe UI 9pt = 12 px, but FluentWinUI3 controls use
    // Segoe UI Variable 10.5pt = 14 px (the design system's "Windows 14"). So read the style's own default
    // control font through a hidden Label instead of Qt.application.font. QML's font value type already derives
    // pixelSize from pointSize, so no DPI maths is needed.
    readonly property Label _fontProbe: Label { visible: false }
    readonly property real base: {
        const f = _fontProbe.font
        return f.pixelSize > 0 ? f.pixelSize : Math.round(f.pointSize * 96 / 72)
    }
    readonly property real sizeDisplay: base + 14
    readonly property real sizeTitle: base + 6
    readonly property real sizeSubtitle: base + 2
    readonly property real sizeBody: base
    readonly property real sizeCaption: base - 2
    readonly property real sizeOverline: base - 3

    // Font objects: use as `font: Theme.fontTitle`. Qt.font() starts from the application font, so the
    // system family is kept.
    readonly property font fontDisplay: Qt.font({ pixelSize: sizeDisplay, weight: Font.Bold })
    readonly property font fontTitle: Qt.font({ pixelSize: sizeTitle, weight: Font.DemiBold })
    readonly property font fontSubtitle: Qt.font({ pixelSize: sizeSubtitle, weight: Font.DemiBold })
    readonly property font fontBody: Qt.font({ pixelSize: sizeBody, weight: Font.Normal })
    readonly property font fontBodyStrong: Qt.font({ pixelSize: sizeBody, weight: Font.DemiBold })
    readonly property font fontCaption: Qt.font({ pixelSize: sizeCaption, weight: Font.Normal })
    readonly property font fontOverline: Qt.font({ pixelSize: sizeOverline, weight: Font.DemiBold,
                                                    capitalization: Font.AllUppercase, letterSpacing: sizeOverline * 0.06 })
    // Tabular figures for durations/counts: `font.features: Theme.tabular` hmm -- see SelfTest; applied per Text.
    readonly property var tabular: ({ "tnum": 1 })

    // ------------------------------------------------------------------ space, size, radius (3.3)
    readonly property int xxs: 2
    readonly property int xs: 4
    readonly property int s: 8
    readonly property int m: 12
    readonly property int l: 16
    readonly property int xl: 24
    readonly property int xxl: 32
    readonly property int xxxl: 48

    readonly property int sidebarWidth: 232
    readonly property int sidebarRailWidth: 56
    readonly property int transportBarHeight: 80
    readonly property int sidePanelWidth: 320
    readonly property int rowHeight: 40
    readonly property int rowHeightCompact: 32
    readonly property int albumTileMin: 168
    readonly property int windowMinWidth: 880
    readonly property int windowMinHeight: 560
    readonly property int hitTarget: 32
    readonly property int hitTargetTouch: 44

    readonly property int radiusControl: Qt.platform.os === "osx" ? 6 : 4
    readonly property int radiusSurface: Qt.platform.os === "osx" ? 10 : 8
    readonly property int radiusArtSmall: 4
    readonly property int radiusArtLarge: 8

    // ------------------------------------------------------------------ motion
    // OS reduced motion -> 0 for everything except progress indicators (which keep their own animation).
    readonly property int durationFast: reducedMotion ? 0 : 120
    readonly property int durationNormal: reducedMotion ? 0 : 200
    readonly property int durationSlow: reducedMotion ? 0 : 300
    readonly property int easingIn: Easing.OutCubic
    readonly property int easingOut: Easing.InCubic

    // ------------------------------------------------------------------ icons
    readonly property int iconSize: 20
    readonly property int iconSizeDense: 16
    readonly property real iconStroke: 1.75

    // Focus ring (7): 2 px accent ring, offset 2.
    readonly property int focusRingWidth: 2
    readonly property int focusRingOffset: 2
}
