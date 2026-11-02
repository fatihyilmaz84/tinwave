//! Qt Quick Controls style selection (design-system.md section 2, ADR 0007 A1).
//!
//! Windows selects `FluentWinUI3` explicitly: Qt's default there is the `Windows` style, which has no dark mode.
//! macOS selects `macOS` (design-system.md section 2) and everything else `Fusion` (Linux: platform palette). The
//! style must be chosen before the engine loads any QML that imports Qt Quick Controls.

/// Qt's own documented style switch (not a Tinwave flag): honoured in release builds too, so the other styles can be
/// checked (design-system.md section 8).
pub const STYLE_ENV: &str = "QT_QUICK_CONTROLS_STYLE";

/// The style the app selects for `os` (a value of `std::env::consts::OS`).
pub fn default_style_for(os: &str) -> &'static str {
    match os {
        "windows" => "FluentWinUI3",
        // Per design-system.md section 2. Unverified on a Mac until T-0038.
        "macos" => "macOS",
        _ => "Fusion",
    }
}

/// What to pass to `QQuickStyle::set_style`: `None` when the developer override is set (an explicit
/// `set_style` call would take precedence over the environment variable, so we must stay out of the way).
pub fn style_to_select(os: &str, env_override: Option<&str>) -> Option<&'static str> {
    match env_override {
        Some(value) if !value.trim().is_empty() => None,
        _ => Some(default_style_for(os)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_selects_fluent_explicitly() {
        assert_eq!(default_style_for("windows"), "FluentWinUI3");
        assert_eq!(style_to_select("windows", None), Some("FluentWinUI3"));
    }

    #[test]
    fn macos_selects_its_native_style_and_linux_uses_fusion() {
        assert_eq!(default_style_for("macos"), "macOS");
        assert_eq!(default_style_for("linux"), "Fusion");
    }

    #[test]
    fn developer_override_wins() {
        assert_eq!(style_to_select("windows", Some("Basic")), None);
    }

    #[test]
    fn blank_override_is_ignored() {
        assert_eq!(style_to_select("windows", Some("  ")), Some("FluentWinUI3"));
        assert_eq!(style_to_select("linux", Some("")), Some("Fusion"));
    }
}
