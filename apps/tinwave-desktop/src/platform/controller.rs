//! The platform decisions, free of Qt and of any OS: what the OS settings are, whether the sidebar backdrop is on, and
//! whether menus can be native. `crate::app::platform_bridge` owns one [`Controller`] and mirrors [`Controller::state`]
//! into QML properties; the tests here drive it with a fake [`Platform`].
//!
//! Rules (ADR 0007 A2/A3, D-0006):
//! - The backdrop is on only when the OS builds support it **and** transparency effects are on **and** no high-contrast
//!   theme is active **and** the developer switch has not turned it off. Otherwise the solid `sidebar` token shows and
//!   [`Backdrop::Solid`] says why (never a silent fallback).
//! - Native menus are used only when the dark-mode opt-in worked; otherwise window menus (never a light menu in a
//!   dark app).
//! - A failed or unsupported settings read leaves the defaults of [`SystemSettings::default`] and says so in
//!   [`State::settings_status`].

use std::sync::Arc;

use super::{ChangeCallback, MenuScheme, NativeWindow, Platform, PlatformError, SystemSettings};

/// Whether the sidebar shows an OS material behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backdrop {
    /// The OS material is applied; the sidebar and window fills are transparent.
    Active,
    /// The solid `sidebar` token shows, for this expected reason (OS setting, build, developer switch).
    Solid(String),
    /// The solid `sidebar` token shows because an OS call failed: unexpected, and reported on stderr.
    Failed(String),
}

impl Backdrop {
    /// Whether the material is applied.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// One line for diagnostics and the Settings screen: `"mica"`, `"solid: <reason>"` or `"failed: <error>"`.
    pub fn describe(&self) -> String {
        match self {
            Self::Active => "mica".to_owned(),
            Self::Solid(reason) => format!("solid: {reason}"),
            Self::Failed(error) => format!("failed: {error}"),
        }
    }
}

/// Everything the UI mirrors. Compared to decide whether anything changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The OS settings (or the defaults while they cannot be read), with transparency already combined with the
    /// developer switch: `transparency_effects` is what the app acts on.
    pub settings: SystemSettings,
    /// `"live"`, or why the OS could not be asked (then `settings` holds the defaults).
    pub settings_status: String,
    /// The sidebar backdrop.
    pub backdrop: Backdrop,
    /// Menus may be native (`Popup.Native`); false means window menus (`Popup.Window`).
    pub native_menus: bool,
    /// `"native"`, or why menus fall back to windows.
    pub menu_status: String,
}

/// Owns the provider calls and the facts they produced.
pub struct Controller {
    platform: Arc<dyn Platform>,
    os: SystemSettings,
    settings_status: Result<(), PlatformError>,
    transparency_allowed: bool,
    window: Option<NativeWindow>,
    /// Why there is no window to attach to (its Qt platform plugin has no OS handles); `None` while simply waiting.
    detached: Option<String>,
    backdrop: Backdrop,
    scheme: MenuScheme,
    applied_scheme: MenuScheme,
    menus: Result<(), PlatformError>,
}

impl std::fmt::Debug for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Controller")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl Controller {
    /// Reads the OS settings and opts native menus into following the OS scheme (ADR 0007: once, at start-up, before
    /// the first menu exists). There is no window yet, so the backdrop starts solid.
    pub fn new(platform: Arc<dyn Platform>) -> Self {
        let (os, settings_status) = match platform.read() {
            Ok(settings) => (settings, Ok(())),
            Err(error) => (SystemSettings::default(), Err(error)),
        };
        let menus = platform.set_menu_scheme(MenuScheme::FollowSystem);
        Self {
            platform,
            os,
            settings_status,
            transparency_allowed: true,
            window: None,
            detached: None,
            backdrop: Backdrop::Solid("no window yet".to_owned()),
            scheme: MenuScheme::FollowSystem,
            applied_scheme: MenuScheme::FollowSystem,
            menus,
        }
    }

    /// Errors that are not merely "this OS cannot do that": a failed OS call. The caller prints them; everything else is
    /// an expected state that [`state`](Self::state) already describes.
    pub fn failures(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (what, result) in [("settings", &self.settings_status), ("menus", &self.menus)] {
            if let Err(error @ PlatformError::Failed { .. }) = result {
                out.push(format!("{what}: {error}"));
            }
        }
        if let Backdrop::Failed(error) = &self.backdrop {
            out.push(format!("backdrop: {error}"));
        }
        out
    }

    /// The facts as the UI shows them.
    pub fn state(&self) -> State {
        State {
            settings: SystemSettings {
                transparency_effects: self.os.transparency_effects && self.transparency_allowed,
                ..self.os
            },
            settings_status: match (&self.settings_status, &self.detached) {
                (Err(error), _) => error.to_string(),
                // The values were read, but nothing watches the OS for changes.
                (Ok(()), Some(reason)) => reason.clone(),
                (Ok(()), None) => "live".to_owned(),
            },
            backdrop: self.backdrop.clone(),
            native_menus: self.menus.is_ok(),
            menu_status: match &self.menus {
                Ok(()) => "native".to_owned(),
                Err(error) => error.to_string(),
            },
        }
    }

    /// The window the backdrop applies to has been created: start watching it for OS setting changes (`on_change`
    /// runs in its window procedure) and apply the backdrop. Returns whether [`state`](Self::state) changed.
    ///
    /// A failed watch is recorded in `settings_status`: the values then stay as read now and no longer follow the OS.
    pub fn attach_window(&mut self, window: NativeWindow, on_change: ChangeCallback) -> bool {
        let before = self.state();
        self.window = Some(window);
        self.detached = None;
        if let Err(error) = self.platform.watch(window, on_change) {
            self.settings_status = Err(error);
        }
        self.apply_backdrop();
        self.state() != before
    }

    /// The main window exists, but it is not an OS window (`qpa` is the Qt platform plugin, e.g. `offscreen`): nothing
    /// is watched and the backdrop stays solid, and [`state`](Self::state) says so instead of reporting a failed OS
    /// call. Returns whether the state changed.
    pub fn window_not_attached(&mut self, qpa: &str) -> bool {
        let before = self.state();
        self.window = None;
        self.detached = Some(format!("not attached (QPA {qpa})"));
        self.apply_backdrop();
        self.state() != before
    }

    /// The OS said something changed: read again and re-apply what depends on it. Returns whether the state changed.
    pub fn refresh(&mut self) -> bool {
        let before = self.state();
        match self.platform.read() {
            Ok(settings) => {
                self.os = settings;
                self.settings_status = Ok(());
            }
            // Keep the last known values; the status tells why they may be stale.
            Err(error) => self.settings_status = Err(error),
        }
        self.apply_menu_scheme(true);
        self.apply_backdrop();
        self.state() != before
    }

    /// Developer switch (`--transparency=off`): `false` forces the solid sidebar whatever the OS says. The OS setting
    /// itself is never touched. Returns whether the state changed.
    pub fn set_transparency_allowed(&mut self, allowed: bool) -> bool {
        let before = self.state();
        self.transparency_allowed = allowed;
        self.apply_backdrop();
        self.state() != before
    }

    /// The app's own colour scheme changed (`Theme.schemeOverride`): native menus follow it. Returns whether the
    /// state changed.
    pub fn set_menu_scheme(&mut self, scheme: MenuScheme) -> bool {
        let before = self.state();
        self.scheme = scheme;
        self.apply_menu_scheme(false);
        self.state() != before
    }

    /// The scheme native menus should draw in. High contrast always follows the system: its palette is not a
    /// light or dark scheme.
    fn wanted_menu_scheme(&self) -> MenuScheme {
        if self.os.high_contrast {
            MenuScheme::FollowSystem
        } else {
            self.scheme
        }
    }

    /// `os_changed`: the OS reported a change (settings, theme or colours, which includes its light/dark scheme).
    fn apply_menu_scheme(&mut self, os_changed: bool) {
        let wanted = self.wanted_menu_scheme();
        // While menus follow the OS, an OS change is applied again: the menu themes the OS cached for the old scheme
        // are flushed (`FlushMenuThemes`), so an open-later native menu is not light inside a dark app. A forced
        // Dark/Light ignores the OS scheme, so it needs nothing. The OS message does not say what changed and the
        // call is cheap and idempotent, so every OS change counts.
        let reflush = os_changed && wanted == MenuScheme::FollowSystem;
        // Retry while the opt-in has not worked; otherwise only when the scheme changed (or must be re-flushed).
        if wanted == self.applied_scheme && self.menus.is_ok() && !reflush {
            return;
        }
        self.menus = self.platform.set_menu_scheme(wanted);
        if self.menus.is_ok() {
            self.applied_scheme = wanted;
        }
    }

    /// Why the backdrop must not show right now, from what the OS and the developer switch say.
    fn backdrop_blocker(&self) -> Option<&'static str> {
        if !self.transparency_allowed {
            Some("turned off with --transparency=off")
        } else if !self.os.transparency_effects {
            Some("transparency effects are off in the OS")
        } else if self.os.high_contrast {
            Some("a high-contrast theme is active")
        } else {
            None
        }
    }

    fn apply_backdrop(&mut self) {
        let Some(window) = self.window else {
            // Without a window there is nothing to turn on or off: that is the reason, whatever the OS settings say.
            self.backdrop = Backdrop::Solid(
                self.detached
                    .clone()
                    .unwrap_or_else(|| "no window yet".to_owned()),
            );
            return;
        };
        if let Some(reason) = self.backdrop_blocker() {
            // Take the material off only if we put it on; a window that never had it needs no OS call.
            self.backdrop = if self.backdrop.is_active() {
                match self.platform.set_backdrop(window, false) {
                    Ok(()) => Backdrop::Solid(reason.to_owned()),
                    Err(error) => {
                        Backdrop::Failed(format!("{reason}; removing the material: {error}"))
                    }
                }
            } else {
                Backdrop::Solid(reason.to_owned())
            };
            return;
        }
        // Applying again is harmless and also repairs a material the OS dropped.
        self.backdrop = match self.platform.set_backdrop(window, true) {
            Ok(()) => Backdrop::Active,
            Err(error @ PlatformError::Unsupported(_)) => Backdrop::Solid(error.to_string()),
            Err(error @ PlatformError::Failed { .. }) => Backdrop::Failed(error.to_string()),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{MenuStyle, SettingsReader, SettingsWatcher, WindowEffects};
    use std::sync::Mutex;

    /// A platform whose answers the test sets and whose calls it can read back.
    #[derive(Default)]
    struct Fake {
        inner: Mutex<FakeInner>,
    }

    struct FakeInner {
        settings: Result<SystemSettings, PlatformError>,
        backdrop: Result<(), PlatformError>,
        menus: Result<(), PlatformError>,
        watch: Result<(), PlatformError>,
        backdrop_calls: Vec<bool>,
        menu_calls: Vec<MenuScheme>,
        watched: Vec<NativeWindow>,
        callback: Option<ChangeCallback>,
    }

    impl Default for FakeInner {
        fn default() -> Self {
            Self {
                settings: Ok(SystemSettings::default()),
                backdrop: Ok(()),
                menus: Ok(()),
                watch: Ok(()),
                backdrop_calls: Vec::new(),
                menu_calls: Vec::new(),
                watched: Vec::new(),
                callback: None,
            }
        }
    }

    impl Fake {
        fn set(&self, f: impl FnOnce(&mut FakeInner)) {
            f(&mut self.inner.lock().expect("fake lock"));
        }
        fn get<T>(&self, f: impl FnOnce(&FakeInner) -> T) -> T {
            f(&self.inner.lock().expect("fake lock"))
        }
    }

    impl SettingsReader for Fake {
        fn read(&self) -> Result<SystemSettings, PlatformError> {
            self.get(|i| i.settings.clone())
        }
    }
    impl SettingsWatcher for Fake {
        fn watch(
            &self,
            window: NativeWindow,
            on_change: ChangeCallback,
        ) -> Result<(), PlatformError> {
            self.set(|i| {
                i.watched.push(window);
                i.callback = Some(on_change);
            });
            self.get(|i| i.watch.clone())
        }
    }
    impl WindowEffects for Fake {
        fn prepare_process(&self) -> Result<(), PlatformError> {
            Ok(())
        }
        fn set_backdrop(&self, _window: NativeWindow, on: bool) -> Result<(), PlatformError> {
            self.set(|i| i.backdrop_calls.push(on));
            self.get(|i| i.backdrop.clone())
        }
    }
    impl MenuStyle for Fake {
        fn set_menu_scheme(&self, scheme: MenuScheme) -> Result<(), PlatformError> {
            self.set(|i| i.menu_calls.push(scheme));
            self.get(|i| i.menus.clone())
        }
    }

    const WINDOW: NativeWindow = NativeWindow(0x1234);

    fn controller(fake: &Arc<Fake>) -> Controller {
        Controller::new(Arc::<Fake>::clone(fake))
    }

    fn attached(fake: &Arc<Fake>) -> Controller {
        let mut c = controller(fake);
        c.attach_window(WINDOW, Arc::new(|| {}));
        c
    }

    #[test]
    fn start_up_opts_menus_into_the_os_scheme_once() {
        let fake = Arc::new(Fake::default());
        let c = controller(&fake);
        assert_eq!(
            fake.get(|i| i.menu_calls.clone()),
            vec![MenuScheme::FollowSystem]
        );
        let state = c.state();
        assert!(state.native_menus);
        assert_eq!(state.menu_status, "native");
        assert_eq!(state.settings_status, "live");
        assert!(!state.backdrop.is_active(), "no window yet");
    }

    #[test]
    fn menus_fall_back_to_windows_when_the_opt_in_fails() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| i.menus = Err(PlatformError::unsupported("uxtheme ordinal 135 is missing")));
        let c = controller(&fake);
        let state = c.state();
        assert!(!state.native_menus);
        assert_eq!(
            state.menu_status,
            "unsupported: uxtheme ordinal 135 is missing"
        );
    }

    #[test]
    fn menus_follow_the_app_scheme_override_and_system_again() {
        let fake = Arc::new(Fake::default());
        let mut c = controller(&fake);
        assert!(
            !c.set_menu_scheme(MenuScheme::Dark),
            "native stays native: no visible change"
        );
        c.set_menu_scheme(MenuScheme::Light);
        c.set_menu_scheme(MenuScheme::FollowSystem);
        assert_eq!(
            fake.get(|i| i.menu_calls.clone()),
            vec![
                MenuScheme::FollowSystem,
                MenuScheme::Dark,
                MenuScheme::Light,
                MenuScheme::FollowSystem
            ]
        );
    }

    #[test]
    fn repeating_the_same_menu_scheme_does_not_call_the_os_again() {
        let fake = Arc::new(Fake::default());
        let mut c = controller(&fake);
        c.set_menu_scheme(MenuScheme::Dark);
        c.set_menu_scheme(MenuScheme::Dark);
        assert_eq!(fake.get(|i| i.menu_calls.len()), 2);
    }

    #[test]
    fn high_contrast_menus_follow_the_system_whatever_the_override() {
        let fake = Arc::new(Fake::default());
        let mut c = controller(&fake);
        c.set_menu_scheme(MenuScheme::Dark);
        fake.set(|i| {
            i.settings = Ok(SystemSettings {
                high_contrast: true,
                ..SystemSettings::default()
            });
        });
        c.refresh();
        assert_eq!(
            fake.get(|i| i.menu_calls.last().copied()),
            Some(MenuScheme::FollowSystem)
        );
        fake.set(|i| i.settings = Ok(SystemSettings::default()));
        c.refresh();
        assert_eq!(
            fake.get(|i| i.menu_calls.last().copied()),
            Some(MenuScheme::Dark)
        );
    }

    #[test]
    fn an_os_change_while_menus_follow_the_system_flushes_them_again() {
        let fake = Arc::new(Fake::default());
        let mut c = attached(&fake);
        assert_eq!(
            fake.get(|i| i.menu_calls.clone()),
            vec![MenuScheme::FollowSystem]
        );
        // The OS switched light/dark: its message reaches the callback, the controller refreshes.
        c.refresh();
        assert_eq!(
            fake.get(|i| i.menu_calls.clone()),
            vec![MenuScheme::FollowSystem, MenuScheme::FollowSystem],
            "the opt-in and flush are applied again for the new OS scheme"
        );
        c.refresh();
        assert_eq!(fake.get(|i| i.menu_calls.len()), 3, "every OS change");
        assert!(c.state().native_menus, "still native");
    }

    #[test]
    fn an_os_change_does_not_touch_menus_forced_to_a_scheme() {
        let fake = Arc::new(Fake::default());
        let mut c = attached(&fake);
        c.set_menu_scheme(MenuScheme::Dark);
        assert_eq!(fake.get(|i| i.menu_calls.len()), 2);
        c.refresh();
        c.refresh();
        assert_eq!(
            fake.get(|i| i.menu_calls.len()),
            2,
            "a forced scheme ignores the OS scheme"
        );
    }

    #[test]
    fn a_window_that_is_not_an_os_window_is_reported_not_attached_and_nothing_is_called() {
        let fake = Arc::new(Fake::default());
        let mut c = controller(&fake);
        assert!(c.window_not_attached("offscreen"));
        let state = c.state();
        assert_eq!(state.settings_status, "not attached (QPA offscreen)");
        assert_eq!(
            state.backdrop,
            Backdrop::Solid("not attached (QPA offscreen)".to_owned())
        );
        assert!(c.failures().is_empty(), "expected, not a failure");
        assert!(fake.get(|i| i.watched.is_empty() && i.backdrop_calls.is_empty()));
        // Refreshing keeps the reason; the OS is still read.
        c.refresh();
        assert_eq!(c.state().settings_status, "not attached (QPA offscreen)");
        assert_eq!(
            c.state().backdrop.describe(),
            "solid: not attached (QPA offscreen)"
        );
    }

    #[test]
    fn an_unreadable_os_is_reported_before_not_attached() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| i.settings = Err(PlatformError::unsupported("no bridge")));
        let mut c = controller(&fake);
        c.window_not_attached("offscreen");
        assert_eq!(c.state().settings_status, "unsupported: no bridge");
    }

    #[test]
    fn a_menu_opt_in_that_failed_is_retried_on_the_next_refresh() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.menus = Err(PlatformError::Failed {
                call: "GetProcAddress",
                code: 127,
            })
        });
        let mut c = controller(&fake);
        assert!(!c.state().native_menus);
        fake.set(|i| i.menus = Ok(()));
        assert!(c.refresh());
        assert!(c.state().native_menus);
    }

    #[test]
    fn attaching_a_window_watches_it_and_turns_the_backdrop_on() {
        let fake = Arc::new(Fake::default());
        let mut c = controller(&fake);
        assert!(c.attach_window(WINDOW, Arc::new(|| {})));
        assert_eq!(fake.get(|i| i.watched.clone()), vec![WINDOW]);
        assert_eq!(fake.get(|i| i.backdrop_calls.clone()), vec![true]);
        assert!(c.state().backdrop.is_active());
        assert_eq!(c.state().backdrop.describe(), "mica");
    }

    #[test]
    fn an_unsupported_os_build_keeps_the_sidebar_solid_and_says_why() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.backdrop = Err(PlatformError::unsupported(
                "Windows build 19045 is older than 22621",
            ))
        });
        let c = attached(&fake);
        assert_eq!(
            c.state().backdrop,
            Backdrop::Solid("unsupported: Windows build 19045 is older than 22621".to_owned())
        );
    }

    #[test]
    fn a_failed_backdrop_call_keeps_the_sidebar_solid_and_is_reported_as_a_failure() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.backdrop = Err(PlatformError::Failed {
                call: "DwmSetWindowAttribute",
                code: 0x8007_0057,
            })
        });
        let c = attached(&fake);
        assert!(!c.state().backdrop.is_active());
        assert_eq!(
            c.state().backdrop.describe(),
            "failed: DwmSetWindowAttribute failed (0x80070057)"
        );
        assert_eq!(
            c.failures(),
            vec!["backdrop: DwmSetWindowAttribute failed (0x80070057)"]
        );
    }

    #[test]
    fn expected_unsupported_answers_are_not_failures() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.settings = Err(PlatformError::unsupported("no bridge"));
            i.menus = Err(PlatformError::unsupported("no bridge"));
            i.watch = Err(PlatformError::unsupported("no bridge"));
            i.backdrop = Err(PlatformError::unsupported("no bridge"));
        });
        let c = attached(&fake);
        assert!(c.failures().is_empty());
    }

    #[test]
    fn failed_settings_and_menu_calls_are_failures() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.settings = Err(PlatformError::Failed {
                call: "RegGetValueW",
                code: 5,
            });
            i.menus = Err(PlatformError::Failed {
                call: "LoadLibraryExW",
                code: 126,
            });
        });
        let c = controller(&fake);
        assert_eq!(
            c.failures(),
            vec![
                "settings: RegGetValueW failed (0x00000005)".to_owned(),
                "menus: LoadLibraryExW failed (0x0000007e)".to_owned(),
            ]
        );
    }

    #[test]
    fn transparency_off_in_the_os_means_no_backdrop_and_it_returns_when_switched_on() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.settings = Ok(SystemSettings {
                transparency_effects: false,
                ..SystemSettings::default()
            });
        });
        let mut c = attached(&fake);
        assert_eq!(
            c.state().backdrop,
            Backdrop::Solid("transparency effects are off in the OS".to_owned())
        );
        assert!(!c.state().settings.transparency_effects);
        assert!(
            fake.get(|i| i.backdrop_calls.is_empty()),
            "never turned on, so nothing to turn off"
        );

        fake.set(|i| i.settings = Ok(SystemSettings::default()));
        assert!(c.refresh());
        assert!(c.state().backdrop.is_active());
    }

    #[test]
    fn high_contrast_removes_an_active_backdrop_and_it_returns_afterwards() {
        let fake = Arc::new(Fake::default());
        let mut c = attached(&fake);
        assert!(c.state().backdrop.is_active());

        fake.set(|i| {
            i.settings = Ok(SystemSettings {
                high_contrast: true,
                ..SystemSettings::default()
            });
        });
        assert!(c.refresh());
        assert_eq!(
            c.state().backdrop,
            Backdrop::Solid("a high-contrast theme is active".to_owned())
        );
        assert!(c.state().settings.high_contrast);
        assert_eq!(fake.get(|i| i.backdrop_calls.clone()), vec![true, false]);

        fake.set(|i| i.settings = Ok(SystemSettings::default()));
        assert!(c.refresh());
        assert!(c.state().backdrop.is_active());
        assert_eq!(
            fake.get(|i| i.backdrop_calls.clone()),
            vec![true, false, true]
        );
    }

    #[test]
    fn the_developer_switch_forces_solid_without_touching_the_os_setting() {
        let fake = Arc::new(Fake::default());
        let mut c = attached(&fake);
        assert!(c.set_transparency_allowed(false));
        let state = c.state();
        assert_eq!(
            state.backdrop,
            Backdrop::Solid("turned off with --transparency=off".to_owned())
        );
        assert!(
            !state.settings.transparency_effects,
            "the app acts on the combined value"
        );
        assert_eq!(fake.get(|i| i.backdrop_calls.clone()), vec![true, false]);

        assert!(c.set_transparency_allowed(true));
        assert!(c.state().backdrop.is_active());
    }

    #[test]
    fn the_developer_switch_before_the_window_exists_never_turns_the_material_on() {
        let fake = Arc::new(Fake::default());
        let mut c = controller(&fake);
        c.set_transparency_allowed(false);
        c.attach_window(WINDOW, Arc::new(|| {}));
        assert!(fake.get(|i| i.backdrop_calls.is_empty()));
    }

    #[test]
    fn a_failure_removing_the_material_is_reported() {
        let fake = Arc::new(Fake::default());
        let mut c = attached(&fake);
        fake.set(|i| {
            i.backdrop = Err(PlatformError::Failed {
                call: "DwmExtendFrameIntoClientArea",
                code: 5,
            })
        });
        c.set_transparency_allowed(false);
        assert_eq!(
            c.state().backdrop.describe(),
            "failed: turned off with --transparency=off; removing the material: \
             DwmExtendFrameIntoClientArea failed (0x00000005)"
        );
    }

    #[test]
    fn refresh_reports_a_change_only_when_a_value_changed() {
        let fake = Arc::new(Fake::default());
        let mut c = attached(&fake);
        assert!(!c.refresh(), "same values: nothing to publish");
        fake.set(|i| {
            i.settings = Ok(SystemSettings {
                reduced_motion: true,
                ..SystemSettings::default()
            });
        });
        assert!(c.refresh());
        assert!(c.state().settings.reduced_motion);
        assert!(!c.refresh());
    }

    #[test]
    fn an_unreadable_os_keeps_the_defaults_and_says_so() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.settings = Err(PlatformError::unsupported(
                "no platform bridge for linux yet",
            ))
        });
        let c = controller(&fake);
        let state = c.state();
        assert_eq!(state.settings, SystemSettings::default());
        assert_eq!(
            state.settings_status,
            "unsupported: no platform bridge for linux yet"
        );
    }

    #[test]
    fn a_failed_read_after_a_good_one_keeps_the_last_values_and_flags_them() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.settings = Ok(SystemSettings {
                reduced_motion: true,
                ..SystemSettings::default()
            });
        });
        let mut c = controller(&fake);
        fake.set(|i| {
            i.settings = Err(PlatformError::Failed {
                call: "SystemParametersInfoW",
                code: 5,
            })
        });
        assert!(c.refresh());
        let state = c.state();
        assert!(state.settings.reduced_motion, "last known value kept");
        assert_eq!(
            state.settings_status,
            "SystemParametersInfoW failed (0x00000005)"
        );
    }

    #[test]
    fn a_failed_watch_is_flagged_but_the_backdrop_still_applies() {
        let fake = Arc::new(Fake::default());
        fake.set(|i| {
            i.watch = Err(PlatformError::Failed {
                call: "SetWindowLongPtrW",
                code: 1400,
            })
        });
        let c = attached(&fake);
        assert_eq!(
            c.state().settings_status,
            "SetWindowLongPtrW failed (0x00000578)"
        );
        assert!(c.state().backdrop.is_active());
    }

    #[test]
    fn the_callback_handed_to_the_watcher_is_the_one_the_os_will_call() {
        let fake = Arc::new(Fake::default());
        let mut c = controller(&fake);
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        c.attach_window(
            WINDOW,
            Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        );
        let callback = fake
            .get(|i| i.callback.clone())
            .expect("the watcher got a callback");
        callback();
        callback();
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
