//! The Windows implementation of the platform traits, and the only module of the platform layer with `unsafe`
//! (every block carries a `SAFETY` comment). Ported from the T-0011 prototype (`spikes/T-0011-native-styles/host/`).
//!
//! Nothing here changes a Windows setting: the calls read (`SystemParametersInfo` queries, one registry value), change
//! this process (an environment variable, the `uxtheme` app mode), or change the look of our own window (DWM
//! attributes).
//!
//! # The undocumented `uxtheme` ordinals (ADR 0007 A2, D-0006)
//!
//! Windows has no public way to make native Win32 menus follow dark mode. The widely used workaround is `uxtheme.dll`
//! ordinal 135 (`SetPreferredAppMode`, `0` default, `1` allow dark = follow the OS, `2` force dark, `3` force light)
//! and ordinal 136 (`FlushMenuThemes`). **They are undocumented and Microsoft may change or remove them.** Containment:
//! they are looked up at run time (never linked), only on builds where ordinal 135 is `SetPreferredAppMode`
//! (>= 18362; on earlier builds it is a different function taking a `BOOL`), and every missing symbol is an error
//! that makes the UI use `Popup.Window` menus instead (the controller does this automatically). Verified on Windows 11
//! build 26200.
//!
//! # Window subclassing
//!
//! Qt delivers no Rust-visible native events, so [`SettingsWatcher::watch`] subclasses the window procedure of the
//! window it is given (`SetWindowLongPtrW`). The OS broadcasts `WM_SETTINGCHANGE` to every top-level window, and the
//! prototype's test (`SendMessage` to its own window, never a broadcast) reached its filter the same way; the tests
//! here do likewise against a message-only window.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::mem::size_of;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ::windows::core::{w, PCSTR, PWSTR};
use ::windows::Wdk::System::SystemServices::RtlGetVersion;
use ::windows::Win32::Foundation::{
    GetLastError, SetLastError, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HMODULE, HWND, LPARAM,
    LRESULT, WIN32_ERROR, WPARAM,
};
use ::windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_NONE,
    DWMWA_SYSTEMBACKDROP_TYPE, DWM_SYSTEMBACKDROP_TYPE,
};
use ::windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use ::windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use ::windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use ::windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW, HIGHCONTRASTW_FLAGS};
use ::windows::Win32::UI::Controls::MARGINS;
use ::windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, IsWindow, SetWindowLongPtrW, SystemParametersInfoW,
    GWLP_WNDPROC, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_NCDESTROY, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE,
    WM_THEMECHANGED, WNDPROC,
};

use super::{
    ChangeCallback, MenuScheme, MenuStyle, NativeWindow, PlatformError, SettingsReader,
    SettingsWatcher, SystemSettings, WindowEffects,
};

/// First Windows 11 build with the public system-backdrop attribute (22H2).
pub const MICA_MIN_BUILD: u32 = 22621;

/// First build on which `uxtheme` ordinal 135 is `SetPreferredAppMode(int)` (Windows 10 1903).
pub const MENU_OPT_IN_MIN_BUILD: u32 = 18362;

/// Set (to `1`) before the Qt application exists: Qt then adds `WS_EX_NOREDIRECTIONBITMAP`, without which the window's
/// opaque redirection surface hides the backdrop (ADR 0007 A3).
pub const REDIRECTION_SURFACE_ENV: &str = "QT_QPA_DISABLE_REDIRECTION_SURFACE";

// The C runtime's environment functions. Qt (a DLL built against the shared UCRT, like Rust's std) reads its environment
// variables through the CRT's own copy, which `SetEnvironmentVariableW` (what `std::env::set_var` calls) does not
// update: a variable set that way is invisible to Qt, and Mica silently fails (measured: a white sidebar). `_putenv_s`
// updates both copies.
extern "C" {
    fn getenv(name: *const c_char) -> *const c_char;
    fn _putenv_s(name: *const c_char, value: *const c_char) -> c_int;
}

/// What Qt sees for the environment variable `name` (the CRT's copy).
fn crt_env(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    // SAFETY: `name` is a live NUL-terminated string. `getenv` returns null or a pointer to a NUL-terminated string
    // owned by the CRT; it is copied out immediately, before anything can change the environment.
    let value = unsafe {
        let ptr = getenv(name.as_ptr());
        if ptr.is_null() {
            return None;
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    };
    Some(value)
}

/// Sets the environment variable `name` for this process in both the CRT's copy and the OS's. Only for set-up before
/// other threads exist (the CRT environment is not thread-safe to modify).
fn set_crt_env(name: &str, value: &str) -> Result<(), PlatformError> {
    let invalid = || PlatformError::Failed {
        call: "_putenv_s",
        code: 22, // EINVAL: an embedded NUL
    };
    let (name, value) = (
        CString::new(name).map_err(|_| invalid())?,
        CString::new(value).map_err(|_| invalid())?,
    );
    // SAFETY: both arguments are live NUL-terminated strings; `_putenv_s` copies them.
    let errno = unsafe { _putenv_s(name.as_ptr(), value.as_ptr()) };
    if errno == 0 {
        Ok(())
    } else {
        Err(PlatformError::Failed {
            call: "_putenv_s",
            code: errno as u32,
        })
    }
}

/// `WM_DWMCOLORIZATIONCOLORCHANGED` (the accent colour changed); not in the `windows` crate's feature set we use.
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;

/// `uxtheme` ordinal 135 on builds >= [`MENU_OPT_IN_MIN_BUILD`]: returns the previous mode.
type SetPreferredAppModeFn = unsafe extern "system" fn(i32) -> i32;
/// `uxtheme` ordinal 136.
type FlushMenuThemesFn = unsafe extern "system" fn();

/// The Windows providers. Cheap to construct; holds only the OS build number.
#[derive(Debug, Clone, Copy)]
pub struct WindowsPlatform {
    /// `None` when `RtlGetVersion` failed: every feature that depends on the build is then unsupported.
    build: Option<u32>,
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsPlatform {
    /// For the OS this process runs on.
    pub fn new() -> Self {
        Self { build: os_build() }
    }

    /// For a given build number (tests).
    pub fn with_build(build: Option<u32>) -> Self {
        Self { build }
    }
}

/// `Ok` when `build` is at least `min`; otherwise the [`PlatformError::Unsupported`] naming `what` and both numbers.
fn build_supports(build: Option<u32>, min: u32, what: &str) -> Result<(), PlatformError> {
    match build {
        Some(build) if build >= min => Ok(()),
        Some(build) => Err(PlatformError::unsupported(format!(
            "{what} needs Windows build {min} or newer (this is {build})"
        ))),
        None => Err(PlatformError::unsupported(format!(
            "{what}: the Windows build number could not be read"
        ))),
    }
}

fn failed(call: &'static str, error: &::windows::core::Error) -> PlatformError {
    // `HRESULT` is signed; the bit pattern is what is printed and compared.
    PlatformError::Failed {
        call,
        code: error.code().0 as u32,
    }
}

/// The Windows build number (`RtlGetVersion` is not subject to the manifest-based version lie of `GetVersionEx`).
fn os_build() -> Option<u32> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..OSVERSIONINFOW::default()
    };
    // SAFETY: `info` is a live, writable OSVERSIONINFOW whose `dwOSVersionInfoSize` is set, as RtlGetVersion requires.
    let status = unsafe { RtlGetVersion(&mut info) };
    status.is_ok().then_some(info.dwBuildNumber)
}

/// The window behind `window`, or a [`PlatformError::Failed`] when it is not a live window of this process's
/// thread-agnostic handle table (a stale or foreign number must not reach DWM).
fn live_window(window: NativeWindow) -> Result<HWND, PlatformError> {
    let hwnd = HWND(window.0 as *mut c_void);
    // SAFETY: IsWindow accepts any value and only answers whether it is a window handle.
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Ok(hwnd)
    } else {
        Err(PlatformError::Failed {
            call: "IsWindow",
            code: 1400, // ERROR_INVALID_WINDOW_HANDLE
        })
    }
}

// ----------------------------------------------------------------------------------------------- SettingsReader

/// "Animation effects" in Settings, Accessibility, Visual effects.
fn client_area_animation() -> Result<bool, PlatformError> {
    let mut enabled = ::windows::core::BOOL(0);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through `pvparam`; `enabled` is a live BOOL and no size is
    // passed because the action takes none.
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|e| failed("SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)", &e))?;
    Ok(enabled.as_bool())
}

/// A high-contrast theme is active.
fn high_contrast_on() -> Result<bool, PlatformError> {
    // The OS writes the active scheme's name into this buffer; we only need the flags.
    let mut scheme = [0u16; 256];
    let mut high_contrast = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        dwFlags: HIGHCONTRASTW_FLAGS(0),
        lpszDefaultScheme: PWSTR(scheme.as_mut_ptr()),
    };
    // SAFETY: SPI_GETHIGHCONTRAST fills a HIGHCONTRASTW whose `cbSize` is set (also passed as `uiparam`); its scheme
    // pointer targets `scheme`, a live 256-element buffer, which outlives the call.
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            high_contrast.cbSize,
            Some((&raw mut high_contrast).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|e| failed("SystemParametersInfoW(SPI_GETHIGHCONTRAST)", &e))?;
    Ok(high_contrast.dwFlags.contains(HCF_HIGHCONTRASTON))
}

/// "Transparency effects" in Settings, Personalization, Colors. The value is absent until the user first changes it,
/// and absent means on.
fn transparency_effects() -> Result<bool, PlatformError> {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: both names are NUL-terminated wide literals; `value` is a live u32 and `size` holds its byte size, as
    // RegGetValueW with RRF_RT_REG_DWORD requires.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast::<c_void>()),
            Some(&raw mut size),
        )
    };
    match status {
        ERROR_SUCCESS => Ok(value != 0),
        ERROR_FILE_NOT_FOUND => Ok(true),
        WIN32_ERROR(code) => Err(PlatformError::Failed {
            call: "RegGetValueW(EnableTransparency)",
            code,
        }),
    }
}

impl SettingsReader for WindowsPlatform {
    fn read(&self) -> Result<SystemSettings, PlatformError> {
        Ok(SystemSettings {
            reduced_motion: !client_area_animation()?,
            high_contrast: high_contrast_on()?,
            transparency_effects: transparency_effects()?,
        })
    }
}

// ---------------------------------------------------------------------------------------------- SettingsWatcher

/// One subclassed window.
struct Hook {
    hwnd: isize,
    /// The window procedure that was in place; every message goes on to it.
    previous: isize,
    on_change: ChangeCallback,
}

/// The subclassed windows. Process-wide because a window procedure has no user data of its own; the lock is never
/// held while a window procedure or a callback runs.
static HOOKS: Mutex<Vec<Hook>> = Mutex::new(Vec::new());

fn hooks() -> MutexGuard<'static, Vec<Hook>> {
    // A panic cannot leave the list half-edited (every edit is a single push, remove or assignment).
    HOOKS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The messages that mean "a setting, the theme or a colour may have changed".
fn is_settings_message(msg: u32) -> bool {
    matches!(
        msg,
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED
    )
}

/// The replacement window procedure: hands every message to the procedure it replaced, then reports settings
/// messages. Runs on the thread that owns the window.
unsafe extern "system" fn hook_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let found = {
        let mut hooks = hooks();
        match hooks.iter().position(|hook| hook.hwnd == hwnd.0 as isize) {
            // The window is going away: forget it, but still let the original procedure see this last message.
            Some(index) if msg == WM_NCDESTROY => Some((hooks.remove(index).previous, None)),
            Some(index) => Some((hooks[index].previous, Some(hooks[index].on_change.clone()))),
            None => None,
        }
    };
    let Some((previous, on_change)) = found else {
        // SAFETY: plain forwarding of this call's own arguments.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    // SAFETY: `previous` is the value SetWindowLongPtrW returned when `watch` installed this procedure, i.e. the
    // window's previous WNDPROC (a non-null function pointer of the right signature).
    let previous = unsafe { std::mem::transmute::<isize, WNDPROC>(previous) };
    // SAFETY: the arguments are those the OS gave this procedure for this window.
    let result = unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) };
    if let Some(on_change) = on_change {
        if is_settings_message(msg) {
            on_change();
        }
    }
    result
}

impl SettingsWatcher for WindowsPlatform {
    fn watch(&self, window: NativeWindow, on_change: ChangeCallback) -> Result<(), PlatformError> {
        let hwnd = live_window(window)?;
        let mut hooks = hooks();
        if let Some(hook) = hooks.iter_mut().find(|hook| hook.hwnd == hwnd.0 as isize) {
            hook.on_change = on_change;
            return Ok(());
        }
        // SAFETY: clearing the thread's last-error value has no preconditions; it lets a 0 return below be told apart
        // from a failure.
        unsafe { SetLastError(ERROR_SUCCESS) };
        // SAFETY: `hwnd` is a live window (checked above) owned by this thread (the caller is the UI thread that
        // created it), and `hook_proc` has exactly the WNDPROC signature.
        let previous =
            unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, hook_proc as usize as isize) };
        if previous == 0 {
            // SAFETY: reads the calling thread's last-error value.
            let code = unsafe { GetLastError() }.0;
            return Err(PlatformError::Failed {
                call: "SetWindowLongPtrW(GWLP_WNDPROC)",
                code,
            });
        }
        hooks.push(Hook {
            hwnd: hwnd.0 as isize,
            previous,
            on_change,
        });
        Ok(())
    }
}

// ------------------------------------------------------------------------------------------------ WindowEffects

impl WindowEffects for WindowsPlatform {
    fn prepare_process(&self) -> Result<(), PlatformError> {
        build_supports(self.build, MICA_MIN_BUILD, "Mica")?;
        // Called from `main`'s thread before Qt exists and before any other thread is started.
        set_crt_env(REDIRECTION_SURFACE_ENV, "1")
    }

    fn set_backdrop(&self, window: NativeWindow, on: bool) -> Result<(), PlatformError> {
        let hwnd = live_window(window)?;
        if on {
            build_supports(self.build, MICA_MIN_BUILD, "Mica")?;
            if crt_env(REDIRECTION_SURFACE_ENV).as_deref() != Some("1") {
                return Err(PlatformError::unsupported(format!(
                    "{REDIRECTION_SURFACE_ENV}=1 was not set before Qt started, so the window would show a white surface"
                )));
            }
        }
        let backdrop: DWM_SYSTEMBACKDROP_TYPE = if on { DWMSBT_MAINWINDOW } else { DWMSBT_NONE };
        // SAFETY: `hwnd` is a live window; DWMWA_SYSTEMBACKDROP_TYPE reads one DWM_SYSTEMBACKDROP_TYPE (an i32) and
        // the size passed is that of the value pointed to.
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&raw const backdrop).cast::<c_void>(),
                size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
            )
        }
        .map_err(|e| failed("DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)", &e))?;
        // The material only shows through where the frame is extended into the client area.
        let margins = if on {
            MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            }
        } else {
            MARGINS::default()
        };
        // SAFETY: `hwnd` is a live window and `margins` is a live MARGINS for the duration of the call.
        unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) }
            .map_err(|e| failed("DwmExtendFrameIntoClientArea", &e))
    }
}

// ----------------------------------------------------------------------------------------------------- MenuStyle

/// `SetPreferredAppMode` argument for `scheme`.
fn preferred_app_mode(scheme: MenuScheme) -> i32 {
    match scheme {
        MenuScheme::FollowSystem => 1, // AllowDark
        MenuScheme::Dark => 2,         // ForceDark
        MenuScheme::Light => 3,        // ForceLight
    }
}

/// Looks `ordinal` up in `module`; `None` when the export does not exist.
fn export_by_ordinal(
    module: HMODULE,
    ordinal: u16,
) -> Option<unsafe extern "system" fn() -> isize> {
    // SAFETY: `module` is a loaded module handle. A pointer whose value is below 0x10000 is, by the GetProcAddress
    // contract, an export ordinal and not a string address.
    unsafe { GetProcAddress(module, PCSTR(usize::from(ordinal) as *const u8)) }
}

impl MenuStyle for WindowsPlatform {
    fn set_menu_scheme(&self, scheme: MenuScheme) -> Result<(), PlatformError> {
        build_supports(self.build, MENU_OPT_IN_MIN_BUILD, "dark native menus")?;
        // SAFETY: a fixed system DLL name, searched in System32 only (no DLL planting); uxtheme is already loaded by
        // Qt, so this only adds a reference that lives as long as the process (the pointers below stay valid).
        let uxtheme =
            unsafe { LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .map_err(|e| failed("LoadLibraryExW(uxtheme.dll)", &e))?;
        let set_mode = export_by_ordinal(uxtheme, 135).ok_or_else(|| {
            PlatformError::unsupported("uxtheme.dll has no export 135 (SetPreferredAppMode)")
        })?;
        let flush = export_by_ordinal(uxtheme, 136).ok_or_else(|| {
            PlatformError::unsupported("uxtheme.dll has no export 136 (FlushMenuThemes)")
        })?;
        // SAFETY: on builds >= MENU_OPT_IN_MIN_BUILD (checked above) ordinal 135 is `int SetPreferredAppMode(int)`
        // and 136 is `void FlushMenuThemes(void)`, both __stdcall (= "system" on x64/arm64). This is undocumented
        // and the reason for the module-level risk note.
        let (set_mode, flush) = unsafe {
            (
                std::mem::transmute::<unsafe extern "system" fn() -> isize, SetPreferredAppModeFn>(
                    set_mode,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, FlushMenuThemesFn>(
                    flush,
                ),
            )
        };
        // SAFETY: the signatures were established above; the mode is one of the four documented-by-use values.
        unsafe {
            set_mode(preferred_app_mode(scheme));
            flush();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use ::windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        WM_NULL,
    };

    /// A message-only window of the system `STATIC` class: invisible, never on the desktop, owned by the test thread.
    struct TestWindow(HWND);

    impl TestWindow {
        fn new() -> Self {
            // SAFETY: standard window creation with a system class and valid constant arguments; message-only
            // windows have no size or position.
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("tinwave platform test"),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    None,
                    None,
                )
            }
            .expect("create a message-only window");
            Self(hwnd)
        }

        fn native(&self) -> NativeWindow {
            NativeWindow(self.0 .0 as usize)
        }

        /// Delivers `msg` to our own window synchronously, like the prototype's test (never a broadcast).
        fn send(&self, msg: u32) {
            // SAFETY: the window is alive and belongs to this thread, so SendMessageW calls its procedure directly.
            unsafe { SendMessageW(self.0, msg, None, None) };
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: destroying a window this thread created; failure only means it is already gone.
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    fn counter() -> (Arc<AtomicUsize>, ChangeCallback) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        (
            hits,
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        )
    }

    #[test]
    fn the_build_gate_names_both_numbers() {
        assert!(build_supports(Some(22621), MICA_MIN_BUILD, "Mica").is_ok());
        assert!(build_supports(Some(26200), MICA_MIN_BUILD, "Mica").is_ok());
        assert_eq!(
            build_supports(Some(19045), MICA_MIN_BUILD, "Mica"),
            Err(PlatformError::unsupported(
                "Mica needs Windows build 22621 or newer (this is 19045)"
            ))
        );
        assert!(matches!(
            build_supports(None, MICA_MIN_BUILD, "Mica"),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn crt_environment_round_trips_and_is_also_visible_to_the_os() {
        // A name nothing else uses; this test process only.
        let name = "TINWAVE_TEST_CRT_ENV";
        assert_eq!(crt_env(name), None);
        set_crt_env(name, "1").expect("set through the CRT");
        assert_eq!(crt_env(name).as_deref(), Some("1"));
        assert_eq!(
            std::env::var(name).as_deref(),
            Ok("1"),
            "the OS copy is updated too"
        );
        assert!(set_crt_env("BAD NAME", "1").is_err());
    }

    #[test]
    fn a_variable_set_only_in_the_os_copy_is_not_what_qt_sees() {
        // The failure that made the sidebar white: `std::env::set_var` does not reach the CRT's copy.
        let name = "TINWAVE_TEST_OS_ONLY_ENV";
        std::env::set_var(name, "1");
        assert_eq!(std::env::var(name).as_deref(), Ok("1"));
        assert_eq!(crt_env(name), None);
    }

    #[test]
    fn the_os_build_is_a_windows_11_or_10_number() {
        let build = os_build().expect("RtlGetVersion works");
        assert!(build >= 10240, "build {build}");
    }

    #[test]
    fn settings_read_from_the_real_os_without_error() {
        // Read-only: asks the OS what it is set to, whatever that is.
        let settings = WindowsPlatform::new()
            .read()
            .expect("the settings are readable");
        // The three reads agree with their separate halves (no torn state).
        assert_eq!(
            settings.reduced_motion,
            !client_area_animation().expect("animation")
        );
        assert_eq!(
            settings.high_contrast,
            high_contrast_on().expect("contrast")
        );
        assert_eq!(
            settings.transparency_effects,
            transparency_effects().expect("transparency")
        );
    }

    #[test]
    fn menu_modes_map_to_the_uxtheme_values() {
        assert_eq!(preferred_app_mode(MenuScheme::FollowSystem), 1);
        assert_eq!(preferred_app_mode(MenuScheme::Dark), 2);
        assert_eq!(preferred_app_mode(MenuScheme::Light), 3);
    }

    #[test]
    fn dark_menus_are_unsupported_before_the_ordinal_changed_meaning() {
        let old = WindowsPlatform::with_build(Some(17763));
        assert!(matches!(
            old.set_menu_scheme(MenuScheme::FollowSystem),
            Err(PlatformError::Unsupported(_))
        ));
        let unknown = WindowsPlatform::with_build(None);
        assert!(matches!(
            unknown.set_menu_scheme(MenuScheme::FollowSystem),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn dark_menus_opt_in_resolves_both_uxtheme_exports_on_this_build() {
        // Changes only this test process's menu mode.
        let platform = WindowsPlatform::new();
        platform
            .set_menu_scheme(MenuScheme::FollowSystem)
            .expect("ordinals 135 and 136 exist on this Windows build");
        platform
            .set_menu_scheme(MenuScheme::Dark)
            .expect("force dark is accepted");
        platform
            .set_menu_scheme(MenuScheme::FollowSystem)
            .expect("and back to following the system");
    }

    #[test]
    fn prepare_process_follows_the_build_gate() {
        assert!(matches!(
            WindowsPlatform::with_build(Some(19045)).prepare_process(),
            Err(PlatformError::Unsupported(_))
        ));
    }

    #[test]
    fn a_backdrop_is_refused_for_a_number_that_is_not_a_window() {
        let platform = WindowsPlatform::with_build(Some(26200));
        assert_eq!(
            platform.set_backdrop(NativeWindow(0), true),
            Err(PlatformError::Failed {
                call: "IsWindow",
                code: 1400
            })
        );
        assert!(platform.watch(NativeWindow(0), Arc::new(|| {})).is_err());
    }

    #[test]
    fn a_backdrop_is_unsupported_on_an_old_build_before_dwm_is_called() {
        let window = TestWindow::new();
        let platform = WindowsPlatform::with_build(Some(19045));
        assert!(matches!(
            platform.set_backdrop(window.native(), true),
            Err(PlatformError::Unsupported(reason)) if reason.contains("22621")
        ));
    }

    #[test]
    fn settings_messages_are_the_four_that_matter() {
        for msg in [
            WM_SETTINGCHANGE,
            WM_SYSCOLORCHANGE,
            WM_THEMECHANGED,
            WM_DWMCOLORIZATIONCOLORCHANGED,
        ] {
            assert!(is_settings_message(msg), "{msg:#x}");
        }
        assert!(!is_settings_message(WM_NULL));
        assert!(!is_settings_message(WM_NCDESTROY));
    }

    #[test]
    fn a_setting_change_sent_to_a_watched_window_reaches_the_callback_once_per_message() {
        let window = TestWindow::new();
        let (hits, callback) = counter();
        WindowsPlatform::new()
            .watch(window.native(), callback)
            .expect("watch the window");

        window.send(WM_NULL);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "unrelated messages are not reported"
        );
        window.send(WM_SETTINGCHANGE);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        window.send(WM_THEMECHANGED);
        window.send(WM_SYSCOLORCHANGE);
        window.send(WM_DWMCOLORIZATIONCOLORCHANGED);
        assert_eq!(hits.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn watching_again_replaces_the_callback_and_does_not_stack_subclasses() {
        let window = TestWindow::new();
        let platform = WindowsPlatform::new();
        let (first, first_cb) = counter();
        let (second, second_cb) = counter();
        platform
            .watch(window.native(), first_cb)
            .expect("first watch");
        platform
            .watch(window.native(), second_cb)
            .expect("second watch");
        window.send(WM_SETTINGCHANGE);
        assert_eq!(first.load(Ordering::SeqCst), 0);
        assert_eq!(second.load(Ordering::SeqCst), 1);
        let count = hooks()
            .iter()
            .filter(|h| h.hwnd == window.0 .0 as isize)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn destroying_a_watched_window_forgets_it() {
        let hwnd;
        {
            let window = TestWindow::new();
            hwnd = window.0 .0 as isize;
            let (_hits, callback) = counter();
            WindowsPlatform::new()
                .watch(window.native(), callback)
                .expect("watch the window");
            assert!(hooks().iter().any(|h| h.hwnd == hwnd));
        }
        assert!(!hooks().iter().any(|h| h.hwnd == hwnd));
    }
}
