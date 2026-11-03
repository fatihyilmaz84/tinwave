//! Operating-system services the UI needs that Qt does not give us on its 6.10 floor, behind small traits.
//!
//! Four providers (ADR 0007, D-0006), each a trait so [`controller::Controller`] is tested with fakes and every OS
//! has its own implementation:
//!
//! | Trait | What | Windows (`windows.rs`) |
//! |---|---|---|
//! | [`SettingsReader`] | reduced motion, high contrast, transparency effects | `SystemParametersInfo`, the `Personalize` registry value |
//! | [`SettingsWatcher`] | "an OS setting changed" for a window | subclass of the window procedure (`WM_SETTINGCHANGE`...) |
//! | [`WindowEffects`] | the sidebar backdrop (Mica) | `DwmSetWindowAttribute` + `DwmExtendFrameIntoClientArea` |
//! | [`MenuStyle`] | the colour scheme of native menus | undocumented `uxtheme` `SetPreferredAppMode` (ordinal 135) |
//!
//! `windows.rs` is the only module of the platform layer that uses `unsafe` (a `SAFETY` comment per block);
//! everywhere else [`unsupported::UnsupportedPlatform`] answers [`PlatformError::Unsupported`] and the UI keeps its
//! solid, default behaviour. Nothing here ever writes an OS setting: every call reads, or changes only this process.
//!
//! The decisions ("is the backdrop wanted now?", "native or window menus?") are in [`controller`], which has no
//! OS or Qt dependency. The QML-facing object in `crate::app::platform_bridge` only mirrors its state.

use std::fmt;
use std::sync::Arc;

pub mod controller;
#[cfg(not(windows))]
pub mod unsupported;
// The one module with `unsafe` (crate lint: `deny`). Named `windows`, so inside this module the crate of the same name
// is reached as `::windows`.
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;

/// The OS-reported state the UI follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemSettings {
    /// "Animation effects" are off: the UI turns every duration but progress indicators to 0.
    pub reduced_motion: bool,
    /// A high-contrast theme is active: the UI uses the system palette only.
    pub high_contrast: bool,
    /// "Transparency effects" are on. Off means no Mica: DWM would paint its own solid colour.
    pub transparency_effects: bool,
}

impl Default for SystemSettings {
    /// What the UI assumes while the OS cannot be asked (an [`Unsupported`](PlatformError::Unsupported) platform or a
    /// failed read): normal motion, normal contrast, transparency as the OS ships it.
    fn default() -> Self {
        Self {
            reduced_motion: false,
            high_contrast: false,
            transparency_effects: true,
        }
    }
}

/// An OS window, by its native handle (`HWND` on Windows). Only [`WindowEffects`] and [`SettingsWatcher`] read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeWindow(pub usize);

/// Which colour scheme native menus draw in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuScheme {
    /// Follow the OS (light or dark, live).
    FollowSystem,
    /// Dark whatever the OS says (the app's own scheme override).
    Dark,
    /// Light whatever the OS says (the app's own scheme override).
    Light,
}

impl MenuScheme {
    /// Maps the value of `Theme.schemeOverride` (`"system"`, `"light"` or `"dark"`); anything else is the system
    /// scheme, exactly as `Theme.setSchemeOverride` treats it.
    pub fn from_override(value: &str) -> Self {
        match value {
            "dark" => Self::Dark,
            "light" => Self::Light,
            _ => Self::FollowSystem,
        }
    }
}

/// Why a provider did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformError {
    /// This OS (or this OS build) cannot do it; the UI uses its solid, default look. Carries the human reason.
    Unsupported(String),
    /// The OS call failed. `code` is the Win32 error or `HRESULT`.
    Failed { call: &'static str, code: u32 },
}

impl PlatformError {
    /// An [`Unsupported`](Self::Unsupported) with its reason.
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported(reason.into())
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(f, "unsupported: {reason}"),
            Self::Failed { call, code } => write!(f, "{call} failed (0x{code:08x})"),
        }
    }
}

impl std::error::Error for PlatformError {}

/// Called on the thread that owns the window when the OS says a setting changed. Must be cheap and must not block:
/// it runs inside the window procedure. It should queue the real work to the UI thread.
pub type ChangeCallback = Arc<dyn Fn() + Send + Sync>;

/// Reads the OS settings the UI follows. Read-only.
pub trait SettingsReader {
    /// The current values.
    fn read(&self) -> Result<SystemSettings, PlatformError>;
}

/// Tells us when an OS setting may have changed.
pub trait SettingsWatcher {
    /// Calls `on_change` whenever the OS tells `window` about a settings, theme or colour change (and once per such
    /// message; it does not say what changed: call [`SettingsReader::read`]). Watching an already watched window
    /// replaces the callback. The watch ends with the window.
    fn watch(&self, window: NativeWindow, on_change: ChangeCallback) -> Result<(), PlatformError>;
}

/// Window backdrop material behind the sidebar (Mica on Windows 11).
pub trait WindowEffects {
    /// Process-wide set-up that must happen **before** the Qt application object exists (Windows: the environment
    /// variable that removes the window's opaque redirection surface). `Ok` means a backdrop can be used later.
    /// Idempotent. Changes this process only.
    fn prepare_process(&self) -> Result<(), PlatformError>;

    /// Turns the backdrop of `window` on or off. [`PlatformError::Unsupported`] when the OS build has none or
    /// [`prepare_process`](Self::prepare_process) did not run.
    fn set_backdrop(&self, window: NativeWindow, on: bool) -> Result<(), PlatformError>;
}

/// The colour scheme of native (OS-drawn) menus.
pub trait MenuStyle {
    /// Makes this process's native menus draw in `scheme`. `Ok` means native menus can be used; any error means they
    /// would draw light inside a dark app, so the UI must use window (Qt-drawn) menus instead. Changes this process
    /// only.
    fn set_menu_scheme(&self, scheme: MenuScheme) -> Result<(), PlatformError>;
}

/// Everything the controller needs from one OS.
pub trait Platform:
    SettingsReader + SettingsWatcher + WindowEffects + MenuStyle + Send + Sync
{
}

impl<T> Platform for T where
    T: SettingsReader + SettingsWatcher + WindowEffects + MenuStyle + Send + Sync
{
}

/// The implementation for the OS this binary runs on.
pub fn current() -> Arc<dyn Platform> {
    #[cfg(windows)]
    {
        Arc::new(self::windows::WindowsPlatform::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(unsupported::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_override_maps_like_the_theme() {
        assert_eq!(MenuScheme::from_override("dark"), MenuScheme::Dark);
        assert_eq!(MenuScheme::from_override("light"), MenuScheme::Light);
        assert_eq!(
            MenuScheme::from_override("system"),
            MenuScheme::FollowSystem
        );
        assert_eq!(MenuScheme::from_override("bogus"), MenuScheme::FollowSystem);
    }

    #[test]
    fn defaults_assume_an_ordinary_desktop() {
        let s = SystemSettings::default();
        assert!(!s.reduced_motion && !s.high_contrast && s.transparency_effects);
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(
            PlatformError::unsupported("Windows build 19045 is older than 22621").to_string(),
            "unsupported: Windows build 19045 is older than 22621"
        );
        assert_eq!(
            PlatformError::Failed {
                call: "DwmSetWindowAttribute",
                code: 0x8007_0057
            }
            .to_string(),
            "DwmSetWindowAttribute failed (0x80070057)"
        );
    }

    // Read-only: asks the OS what it is set to and changes nothing.
    #[cfg(windows)]
    #[test]
    fn current_platform_reads_the_os_on_windows() {
        assert!(current().read().is_ok());
    }

    #[cfg(not(windows))]
    #[test]
    fn current_platform_is_unsupported_elsewhere() {
        assert!(matches!(
            current().read(),
            Err(PlatformError::Unsupported(_))
        ));
    }
}
