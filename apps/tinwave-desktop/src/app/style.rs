//! Qt Quick Controls style selection (design-system.md section 2, ADR 0007 A1).
//!
//! Windows selects `FluentWinUI3` explicitly: Qt's default there is the `Windows` style, which has no dark mode.
//! Everywhere else the shell uses `Fusion` until the per-platform styles get their own tickets. The style must be
//! chosen before the engine loads any QML that imports Qt Quick Controls.

/// Environment variable Qt itself reads to override the style (kept as the developer override, design-system.md section 8).
pub const STYLE_ENV: &str = "QT_QUICK_CONTROLS_STYLE";

/// The style the app selects for `os` (a value of `std::env::consts::OS`).
pub fn default_style_for(os: &str) -> &'static str {
    match os {
        "windows" => "FluentWinUI3",
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
    fn other_platforms_use_fusion() {
        assert_eq!(default_style_for("linux"), "Fusion");
        assert_eq!(default_style_for("macos"), "Fusion");
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
