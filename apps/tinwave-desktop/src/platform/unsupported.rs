//! The platform layer on every OS that has no implementation yet (macOS and Linux today, until their platform tickets).
//! Every provider answers [`PlatformError::Unsupported`], so the UI keeps its solid backdrop, its default settings and
//! window (Qt-drawn) menus. Nothing is faked: the answer names the reason.

use super::{
    ChangeCallback, MenuScheme, MenuStyle, NativeWindow, PlatformError, SettingsReader,
    SettingsWatcher, SystemSettings, WindowEffects,
};

/// Answers [`PlatformError::Unsupported`] to everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedPlatform;

fn unsupported() -> PlatformError {
    PlatformError::unsupported(format!(
        "no platform bridge for {} yet",
        std::env::consts::OS
    ))
}

impl SettingsReader for UnsupportedPlatform {
    fn read(&self) -> Result<SystemSettings, PlatformError> {
        Err(unsupported())
    }
}

impl SettingsWatcher for UnsupportedPlatform {
    fn watch(
        &self,
        _window: NativeWindow,
        _on_change: ChangeCallback,
    ) -> Result<(), PlatformError> {
        Err(unsupported())
    }
}

impl WindowEffects for UnsupportedPlatform {
    fn prepare_process(&self) -> Result<(), PlatformError> {
        Err(unsupported())
    }

    fn set_backdrop(&self, _window: NativeWindow, _on: bool) -> Result<(), PlatformError> {
        Err(unsupported())
    }
}

impl MenuStyle for UnsupportedPlatform {
    fn set_menu_scheme(&self, _scheme: MenuScheme) -> Result<(), PlatformError> {
        Err(unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn every_provider_says_unsupported() {
        let p = UnsupportedPlatform;
        let window = NativeWindow(1);
        assert!(matches!(p.read(), Err(PlatformError::Unsupported(_))));
        assert!(matches!(
            p.watch(window, Arc::new(|| {})),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(
            p.prepare_process(),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(
            p.set_backdrop(window, true),
            Err(PlatformError::Unsupported(_))
        ));
        assert!(matches!(
            p.set_menu_scheme(MenuScheme::Dark),
            Err(PlatformError::Unsupported(_))
        ));
    }
}
