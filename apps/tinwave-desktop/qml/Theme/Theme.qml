// Design tokens (design-system.md section 3). Ported from the T-0011 prototype; T-0035 applied the contrast fixes it found.
//
// Shape: ONE pure function `resolve(dark, accent, hc)` maps (scheme, accent) -> every token. The
// live properties bind to its result, and the same function can be called for arbitrary accents (contrast checks).
// Inputs that come from Qt itself (no bridge needed on Qt >= 6.8):
//   - Application.styleHints.colorScheme   (light/dark, writable: user override == setColorScheme)
//   - SystemPalette.accent / .highlight ... (QPalette::Accent, follows the OS accent on Windows/macOS)
//   - Qt.application.font                   (system font)
// Inputs the platform bridge supplies (T-0036; Qt 6.10 adds Application.styleHints.accessibility, 6.12 motionPreference, so
// the OS is asked directly until the floor makes them enough). Theme itself never imports the bridge, so this file runs
// standalone for the contrast self-check; Main.qml binds them from the `PlatformSettings` singleton:
//   - reducedMotion, highContrast, backdropActive
pragma Singleton
import QtQuick
import QtQuick.Controls

QtObject {
    id: theme

    // ------------------------------------------------------------------ inputs
    // Bridge-supplied (Rust `PlatformSettings`, bound in Main.qml). Never set these from anywhere else.
    property bool reducedMotion: false      // OS "animation effects" off
    property bool highContrast: false       // OS high-contrast theme active
    // Mica / vibrancy is applied to the window: the bridge sets it only when the OS supports it, "Transparency effects"
    // are on, no high-contrast theme is active and no developer switch forced solid (the solid `sidebar` token shows).
    property bool backdropActive: false
    // Whether the OS exposes an accent colour at all. Windows and macOS always do; on Linux only some
    // desktops do (the bridge sets this from the XDG portal); until then assume not and use Tin Teal.
    property bool osAccentAvailable: Qt.platform.os !== "linux"

    // User overrides (Settings). "" / "system" == follow the OS.
    property string accentOverride: ""      // "#rrggbb" or "" (follow OS)
    property string schemeOverride: "system" // "system" | "light" | "dark"

    // Developer switch `--accent=#rrggbb` (and later the Settings accent): "" follows the OS. Anything that is not
    // "#rrggbb" is refused, not guessed at. Returns whether the value was accepted. Never touches the OS accent.
    function setAccentOverride(value) {
        if (value !== "" && !/^#[0-9a-fA-F]{6}$/.test(value)) return false
        accentOverride = value
        return true
    }

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
    function _minContrastOn(fgs, bg) {
        let m = 1000
        for (const f of fgs) m = Math.min(m, contrast(f, bg))
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
    // Best foreground for text ON an accent fill: the first of the scheme default, white, ink, black that reaches 4.5:1
    // on EVERY fill it may sit on (the raw accent and `accentFill`, see resolve()); else the best of them.
    function onAccentFor(fills, preferred) {
        const white = Qt.rgba(1, 1, 1, 1), ink = Qt.color("#0F2F33")
        const cands = [Qt.color(preferred), white, ink, Qt.rgba(0, 0, 0, 1)]
        let best = cands[0], bestC = 0
        for (const c of cands) {
            const v = _minContrast(c, fills)
            if (v >= 4.5) return c
            if (v > bestC) { bestC = v; best = c }
        }
        return best
    }
    // Every surface text can sit on: the four fixed ones, `selected`, and a hovered Tw row over each fixed one
    // (the translucent `hover` overlay composited, 8-bit quantised). Keys are the surface names.
    // `hover` is NEVER composited over `selected`: a selected Tw row shows `selected` only, whether hovered or not
    // (native ItemDelegate keeps its own states), so there is no hover/selected surface to check.
    function surfacesOf(t) {
        const out = {}
        const hv = Qt.color(t.hover)
        for (const k of ["window", "sidebar", "content", "raised"]) {
            const base = Qt.color(t[k])
            out[k] = base
            out["hover/" + k] = mix(base, Qt.rgba(hv.r, hv.g, hv.b, 1), hv.a)
        }
        out.selected = Qt.color(t.selected)
        return out
    }
    function _values(o) { return Object.keys(o).map(k => o[k]) }
    // `selected` = accent over `content` at the nominal strength (selectedAlphaLight/Dark), backed off toward plain
    // `content` just enough that every text token still reaches 4.5:1 on it (a white accent on the dark scheme would
    // otherwise brighten the row past what `textFaint` tolerates). Normal accents are never backed off.
    function _selected(accent, nominal, content, texts) {
        const fits = a => _minContrastOn(texts, over(accent, a, content)) >= 4.5
        if (fits(nominal)) return over(accent, nominal, content)
        let lo = 0, hi = nominal
        for (let i = 0; i < 12; ++i) {
            const mid = (lo + hi) / 2
            if (fits(mid)) lo = mid; else hi = mid
        }
        return over(accent, lo, content)
    }

    // ------------------------------------------------------------------ token resolution
    readonly property color tinTeal: "#0B6E78"
    readonly property color tinTealDark: "#5CC6CF"

    // Fixed values from design-system.md section 3.1. Alpha colours are written as Qt.rgba() on purpose: CSS-style
    // "#RRGGBBAA" is parsed by QML as #AARRGGBB, so a hex-with-alpha must never be copied between the design file and here
    // (design-system.md writes them in QML order, #AARRGGBB, and says so).
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
        t.textFaint = L ? "#64646D" : "#94949C"
        t.srcServer = L ? "#1F5FA8" : "#7BB0EE"
        t.srcDownload = L ? "#2A7240" : "#76C48E"
        t.warning = L ? "#925700" : "#E0A453"
        t.danger = L ? "#B42318" : "#F2877D"
        t.accent = accent
        const texts = ["text", "textMuted", "textFaint", "srcServer", "srcDownload", "warning", "danger"].map(k => Qt.color(t[k]))
        t.selected = _selected(accent, L ? selectedAlphaLight : selectedAlphaDark, Qt.color(t.content), texts)
        // accent used as TEXT (or as a UI glyph / focus ring) must reach 4.5:1 on every surface text can sit on.
        const surfaces = _values(surfacesOf(t))
        t.accentText = ensureContrast(Qt.color(accent), surfaces, !L, 4.5)
        // accentFill: accent used as the FILL of NATIVE accented controls (Button.highlighted, Switch, CheckBox) and as
        // palette.accent / palette.highlight. FluentWinUI3 hard-codes the label colour on those fills (white in light,
        // black in dark -- it ignores palette.highlightedText), so the fill itself must give 4.5:1 against that fixed colour.
        const fixedLabel = Qt.color(L ? "#FFFFFF" : "#000000")
        t.accentFill = ensureContrast(Qt.color(accent), [fixedLabel], !L, 4.5)
        // onAccent: computed text colour for either fill (the raw accent on a Tw fill, accentFill under native controls).
        t.onAccent = onAccentFor([Qt.color(accent), Qt.color(t.accentFill)], L ? "#FFFFFF" : "#0F2F33")
        return t
    }
    // Nominal strength of the `selected` tint. The design file's old hexes encoded ~10 % (light) / ~12 % (dark) of Tin Teal,
    // not the 12 % / 18 % its table claimed; 10 % / 12 % are the rule now.
    readonly property real selectedAlphaLight: 0.10
    readonly property real selectedAlphaDark: 0.12

    // The 15 accents the self-check runs by default (T-0011 SelfTest list): Windows default blue, Tin Teal, the Windows
    // palette picks (yellow, pink, green, red, orange, grey) and pale/dark/neutral extremes.
    readonly property var checkAccents: ["#0067C0", "#0B6E78", "#FFB900", "#E3008C", "#10893E", "#D13438", "#FF8C00", "#767676",
                                         "#00B7C3", "#FFFFFF", "#000000", "#F0E68C", "#7A7574", "#5CC6CF", "#99FF99"]

    // Contrast self-check, the documented gate for every token change (design-system.md 3.1). Resolves every given accent
    // in both schemes and checks: all text tokens >= 4.5:1 on every surface (incl. hovered and selected rows); accentText
    // >= 4.5:1 (it carries accent-coloured text, glyphs and focus rings, so it also clears the 3:1 glyph bar);
    // onAccent >= 4.5:1 on both fills; the style's fixed label >= 4.5:1 on accentFill. Returns { failures, rows }.
    // `accents` defaults to `checkAccents`, the reproducible input of the gate.
    function selfCheck(accents) {
        if (accents === undefined) accents = checkAccents
        const rows = []
        let failures = 0
        const f2 = x => x.toFixed(2)
        const textKeys = ["text", "textMuted", "textFaint", "srcServer", "srcDownload", "warning", "danger"]
        for (const isDark of [false, true]) {
            for (const a of accents) {
                const acc = Qt.color(a)
                const t = resolve(isDark, acc, false)
                const surf = surfacesOf(t)
                let textMin = 1000, textAt = ""
                for (const k of textKeys)
                    for (const sn in surf) {
                        const c = contrast(Qt.color(t[k]), surf[sn])
                        if (c < textMin) { textMin = c; textAt = k + " on " + sn }
                    }
                const accentTextMin = _minContrast(Qt.color(t.accentText), _values(surf))
                const onAccentMin = _minContrast(Qt.color(t.onAccent), [acc, Qt.color(t.accentFill)])
                const fixedLabel = Qt.color(isDark ? "#000000" : "#FFFFFF")
                const fillMin = contrast(Qt.color(t.accentFill), fixedLabel)
                const ok = textMin >= 4.5 && accentTextMin >= 4.5 && onAccentMin >= 4.5 && fillMin >= 4.5
                if (!ok) ++failures
                rows.push((isDark ? "dark  " : "light ") + a + " text>=" + f2(textMin) + " (" + textAt + ") accentText>=" + f2(accentTextMin)
                          + " onAccent>=" + f2(onAccentMin) + " fixedLabel/accentFill=" + f2(fillMin) + " " + (ok ? "ok" : "FAIL"))
            }
        }
        return { failures: failures, rows: rows }
    }

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
    readonly property int windowDefaultWidth: 1200     // first-start size; the last size is remembered later
    readonly property int windowDefaultHeight: 800
    readonly property int windowChromeAllowance: 48    // title bar + frame, subtracted from the available screen height
    // First-start window size: the default, clamped to the available screen (never below the window minimum).
    function firstStartWidth(availableWidth) { return Math.max(windowMinWidth, Math.min(windowDefaultWidth, availableWidth)) }
    function firstStartHeight(availableHeight) {
        return Math.max(windowMinHeight, Math.min(windowDefaultHeight, availableHeight - windowChromeAllowance))
    }
    readonly property int sidebarRowHeight: 34         // canvas nav item; the native ItemDelegate default is 40
    readonly property int hairline: 1                  // separator / outline width (logical px)
    readonly property int hitTarget: 32
    readonly property int hitTargetTouch: 44

    readonly property int radiusControl: Qt.platform.os === "osx" ? 6 : 4
    readonly property int radiusSurface: Qt.platform.os === "osx" ? 10 : 8
    readonly property int radiusContent: radiusSurface // content surface's rounded top-left corner where it meets the sidebar
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

    // Focus ring (7): 2 px ring in `accentText` (>= 3:1 with any accent), offset 2.
    readonly property int focusRingWidth: 2
    readonly property int focusRingOffset: 2
}
