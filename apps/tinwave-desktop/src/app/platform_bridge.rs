//! `PlatformSettings`: the one QML-facing object for the platform layer (`crate::platform`). A QML singleton that
//! mirrors a [`Controller`](crate::platform::controller::Controller)'s state: the OS settings the UI follows (reduced
//! motion, high contrast, transparency), whether the sidebar backdrop (Mica) is on, and whether menus may be native.
//! `Main.qml` binds the first three into `Theme`; `Sidebar.qml` reads `nativeMenus`.
//!
//! It lives in `app/` because cxx-qt-build requires every bridge file of a QML module in one directory (ADR 0008). All
//! decisions are in the controller; this file only translates. Its `unsafe` is the two things a bridge cannot avoid: the
//! raw `QQuickWindow` pointer QML hands to [`attach_window`](qobject::PlatformSettings) and the cxx declarations.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!(<QtQuick/QQuickWindow>);
        /// The window QML passes in. Only its native handle is read.
        type QQuickWindow;
        include!("platform_window.h");
        /// `QWindow::winId()`: the `HWND` on Windows (creates the native window if it does not exist yet). A shim
        /// (`cpp/platform_window.cpp`) because `WId` is `quint64` and cxx's `usize` is `size_t`: those are different
        /// C++ types on 64-bit Linux and macOS, so a direct binding would not compile there.
        fn tw_native_window_handle(window: &QQuickWindow) -> usize;
        /// `QGuiApplication::platformName()`: "windows", "offscreen"... Only some Qt platform plugins hand out real
        /// OS window handles.
        fn tw_qpa_name() -> String;
        /// `QQuickWindow::setDefaultAlphaBuffer`: windows created afterwards get an alpha channel.
        #[Self = "QQuickWindow"]
        #[rust_name = "set_default_alpha_buffer"]
        fn setDefaultAlphaBuffer(use_alpha: bool);
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, reduced_motion, cxx_name = "reducedMotion", READ, NOTIFY)]
        #[qproperty(bool, high_contrast, cxx_name = "highContrast", READ, NOTIFY)]
        #[qproperty(
            bool,
            transparency_effects,
            cxx_name = "transparencyEffects",
            READ,
            NOTIFY
        )]
        #[qproperty(bool, backdrop_active, cxx_name = "backdropActive", READ, NOTIFY)]
        #[qproperty(QString, backdrop_status, cxx_name = "backdropStatus", READ, NOTIFY)]
        #[qproperty(bool, native_menus, cxx_name = "nativeMenus", READ, NOTIFY)]
        #[qproperty(QString, menu_status, cxx_name = "menuStatus", READ, NOTIFY)]
        #[qproperty(QString, settings_status, cxx_name = "settingsStatus", READ, NOTIFY)]
        type PlatformSettings = super::PlatformSettingsRust;
    }

    impl cxx_qt::Threading for PlatformSettings {}

    extern "RustQt" {
        /// The main window exists: watch it for OS setting changes and apply the sidebar backdrop.
        #[qinvokable]
        #[cxx_name = "attachWindow"]
        unsafe fn attach_window(self: Pin<&mut PlatformSettings>, window: *mut QQuickWindow);

        /// Developer switch `--transparency=off`: `false` forces the solid sidebar; the OS setting is untouched.
        #[qinvokable]
        #[cxx_name = "setTransparencyAllowed"]
        fn set_transparency_allowed(self: Pin<&mut PlatformSettings>, allowed: bool);

        /// The app's colour scheme override (`Theme.schemeOverride`: `"system"`, `"light"`, `"dark"`): native menus
        /// follow it.
        #[qinvokable]
        #[cxx_name = "setMenuScheme"]
        fn set_menu_scheme(self: Pin<&mut PlatformSettings>, scheme: &QString);
    }
}

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::platform::controller::{Controller, State};
use crate::platform::{self, MenuScheme, NativeWindow, PlatformError};

/// Makes windows created from now on carry an alpha channel, if this process could be prepared for a backdrop.
/// Must run after [`prepare_process`] succeeded and before the first window exists; harmless otherwise.
fn enable_alpha_buffer() {
    qobject::QQuickWindow::set_default_alpha_buffer(true);
}

/// Process-wide set-up for the sidebar backdrop. Call before the `QGuiApplication` is created.
///
/// Where the OS supports a backdrop this sets what Qt needs before any window exists (on Windows the environment
/// variable `QT_QPA_DISABLE_REDIRECTION_SURFACE=1`, and an alpha buffer for new windows). Where it does not, nothing
/// changes and [`PlatformSettings`](qobject::PlatformSettings) later reports why the sidebar is solid. Returns whether a
/// backdrop is possible in this process.
pub fn prepare_process() -> bool {
    match platform::current().prepare_process() {
        Ok(()) => {
            enable_alpha_buffer();
            true
        }
        // The expected answer on an OS or build without a backdrop: the controller reports the reason to the UI.
        Err(PlatformError::Unsupported(_)) => false,
        Err(error) => {
            eprintln!("tinwave: window backdrop set-up failed, the sidebar stays solid: {error}");
            false
        }
    }
}

/// Rust state of `PlatformSettings`.
pub struct PlatformSettingsRust {
    controller: Controller,
    reduced_motion: bool,
    high_contrast: bool,
    transparency_effects: bool,
    backdrop_active: bool,
    backdrop_status: QString,
    native_menus: bool,
    menu_status: QString,
    settings_status: QString,
}

impl std::fmt::Debug for PlatformSettingsRust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformSettingsRust")
            .field("controller", &self.controller)
            .finish_non_exhaustive()
    }
}

impl Default for PlatformSettingsRust {
    /// Reads the OS and opts native menus into the OS scheme (ADR 0007: at start-up, before the first menu exists).
    /// The properties start from the controller's state, so QML never sees a stale first value.
    fn default() -> Self {
        let controller = Controller::new(platform::current());
        let State {
            settings,
            settings_status,
            backdrop,
            native_menus,
            menu_status,
        } = controller.state();
        Self {
            controller,
            reduced_motion: settings.reduced_motion,
            high_contrast: settings.high_contrast,
            transparency_effects: settings.transparency_effects,
            backdrop_active: backdrop.is_active(),
            backdrop_status: QString::from(backdrop.describe().as_str()),
            native_menus,
            menu_status: QString::from(menu_status.as_str()),
            settings_status: QString::from(settings_status.as_str()),
        }
    }
}

/// Writes `$value` into the field and emits its notify signal, only when it differs.
macro_rules! publish {
    ($this:expr, $field:ident, $notify:ident, $value:expr) => {{
        let value = $value;
        if $this.as_mut().rust().$field != value {
            $this.as_mut().rust_mut().$field = value;
            $this.as_mut().$notify();
        }
    }};
}

impl qobject::PlatformSettings {
    /// Copies the controller's state into the properties, notifying only what changed.
    fn publish_state(mut self: Pin<&mut Self>) {
        let State {
            settings,
            settings_status,
            backdrop,
            native_menus,
            menu_status,
        } = self.rust().controller.state();
        publish!(
            self,
            reduced_motion,
            reduced_motion_changed,
            settings.reduced_motion
        );
        publish!(
            self,
            high_contrast,
            high_contrast_changed,
            settings.high_contrast
        );
        publish!(
            self,
            transparency_effects,
            transparency_effects_changed,
            settings.transparency_effects
        );
        publish!(
            self,
            backdrop_active,
            backdrop_active_changed,
            backdrop.is_active()
        );
        publish!(
            self,
            backdrop_status,
            backdrop_status_changed,
            QString::from(backdrop.describe().as_str())
        );
        publish!(self, native_menus, native_menus_changed, native_menus);
        publish!(
            self,
            menu_status,
            menu_status_changed,
            QString::from(menu_status.as_str())
        );
        publish!(
            self,
            settings_status,
            settings_status_changed,
            QString::from(settings_status.as_str())
        );
    }

    /// QML: the main window exists.
    ///
    /// # Safety
    /// `window` must be null or point to a live `QQuickWindow`; QML passes the `ApplicationWindow` itself.
    unsafe fn attach_window(mut self: Pin<&mut Self>, window: *mut qobject::QQuickWindow) {
        // SAFETY: the caller guarantees `window` is null or live, and it is only read for this call.
        let Some(window) = (unsafe { window.as_ref() }) else {
            eprintln!("tinwave: attachWindow was called without a window");
            return;
        };
        // The offscreen plugin's winId is a counter, not an OS window: never hand such a number to the providers.
        let qpa = qobject::tw_qpa_name();
        if !platform::has_native_window_handles(&qpa) {
            self.as_mut()
                .rust_mut()
                .controller
                .window_not_attached(&qpa);
            self.publish_state();
            return;
        }
        let handle = NativeWindow(qobject::tw_native_window_handle(window));

        // The OS tells the window procedure (on this thread); the work is queued to the event loop. A burst of
        // messages (an accent change sends several) collapses into one refresh.
        let thread = self.qt_thread();
        let queued = Arc::new(AtomicBool::new(false));
        let on_change: platform::ChangeCallback = Arc::new(move || {
            if queued.swap(true, Ordering::SeqCst) {
                return;
            }
            let queued = Arc::clone(&queued);
            // An error means the object is gone (shutdown): nothing left to update.
            let _ = thread.queue(move |mut this| {
                queued.store(false, Ordering::SeqCst);
                let changed = this.as_mut().rust_mut().controller.refresh();
                if changed {
                    this.publish_state();
                }
            });
        });

        let mut this = self.as_mut();
        this.as_mut()
            .rust_mut()
            .controller
            .attach_window(handle, on_change);
        for failure in this.rust().controller.failures() {
            eprintln!("tinwave: platform {failure}");
        }
        this.publish_state();
    }

    /// QML: `--transparency=off`.
    fn set_transparency_allowed(mut self: Pin<&mut Self>, allowed: bool) {
        self.as_mut()
            .rust_mut()
            .controller
            .set_transparency_allowed(allowed);
        self.publish_state();
    }

    /// QML: the app's colour scheme override changed.
    fn set_menu_scheme(mut self: Pin<&mut Self>, scheme: &QString) {
        let scheme = MenuScheme::from_override(&scheme.to_string());
        self.as_mut().rust_mut().controller.set_menu_scheme(scheme);
        self.publish_state();
    }
}
