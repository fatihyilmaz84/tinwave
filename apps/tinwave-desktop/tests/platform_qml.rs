//! The platform bridge as QML sees it, run headless (`QT_QPA_PLATFORM=offscreen`, software rendering): the real window
//! shell is instantiated by a test-only root (`tests/qml/PlatformProbe.qml`) that prints what `PlatformSettings` and
//! `Theme` say. The OS is only read, never changed; the offscreen window has no native handle, so what a real window
//! gets (Mica, the settings watch) is covered by `platform::windows` tests and the PO runbook.
//!
//! The root-URL override only exists in debug builds, so this file does too.
#![cfg(debug_assertions)]

// Referenced so the test binary links the crate (and with it the Rust half of the CXX-Qt bridge); it never calls into it.
use tinwave_desktop as _;

mod common;

use common::probe::{probe as run_probe, Run};

/// Runs the app on the platform probe root with `args`.
fn probe(args: &[&str]) -> Run {
    run_probe("PlatformProbe.qml", args)
}

#[test]
fn theme_follows_the_bridge_and_the_accent_setter_refuses_bad_values() {
    let run = probe(&[]);
    assert_eq!(run.get("themeReducedMotionFollows"), "true");
    assert_eq!(run.get("themeHighContrastFollows"), "true");
    assert_eq!(run.get("themeBackdropFollows"), "true");
    // 120 ms unless the OS has animation effects off (read-only: whatever this machine is set to).
    assert!(
        ["0", "120"].contains(&run.get("durationFast")),
        "durationFast {}",
        run.get("durationFast")
    );
    assert_eq!(run.get("accentIsOverride"), "false", "no --accent given");

    assert_eq!(run.get("bogusAccentAccepted"), "false");
    assert_eq!(run.get("shortAccentAccepted"), "false");
    assert_eq!(run.get("alphaAccentAccepted"), "false");
    assert_eq!(
        run.get("accentAfterRefusals"),
        run.get("accent"),
        "refused values change nothing"
    );
    assert_eq!(run.get("goodAccentAccepted"), "true");
    assert_eq!(run.get("accentAfterGood").to_lowercase(), "#a1b2c3");
    assert_eq!(run.get("resetAccepted"), "true");
    assert_eq!(run.get("accentOverrideAfterReset"), "true");
}

#[cfg(windows)]
#[test]
fn windows_reads_the_os_and_offers_native_menus() {
    let run = probe(&[]);
    // Settings are live unless the window watch failed (the offscreen window has no HWND, so it does).
    assert!(
        run.get("settingsStatus") == "live" || run.get("settingsStatus").contains("IsWindow"),
        "settingsStatus {}",
        run.get("settingsStatus")
    );
    // Windows 10 1903 and newer have the dark-menu opt-in; so does every machine this runs on.
    assert_eq!(
        run.get("nativeMenus"),
        "true",
        "menuStatus {}",
        run.get("menuStatus")
    );
    assert_eq!(run.get("menuStatus"), "native");
    // No real window: the backdrop is not applied and says so (never a silent solid).
    assert_eq!(run.get("backdropActive"), "false");
    assert!(
        run.get("backdropStatus").starts_with("failed: IsWindow")
            || run.get("backdropStatus").starts_with("solid:"),
        "{}",
        run.get("backdropStatus")
    );
}

#[test]
fn developer_switches_change_the_app_in_process_only() {
    let run = probe(&["--accent=#d13438", "--transparency=off"]);
    assert_eq!(run.get("accentIsOverride"), "true");
    assert_eq!(run.get("accent").to_lowercase(), "#d13438");
    assert_eq!(run.get("backdropActive"), "false");
    assert_eq!(
        run.get("backdropStatus"),
        "solid: turned off with --transparency=off"
    );
    assert_eq!(
        run.get("transparencyEffects"),
        "false",
        "the app acts on the combined value"
    );
}

#[test]
fn bad_switch_values_are_reported_and_ignored() {
    let run = probe(&["--accent=teal", "--transparency=maybe"]);
    assert_eq!(run.get("accentIsOverride"), "false");
    assert!(
        run.stderr.contains("--accent expects #rrggbb, got 'teal'"),
        "stderr:\n{}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("--transparency expects off or system, got 'maybe'"),
        "stderr:\n{}",
        run.stderr
    );
}
