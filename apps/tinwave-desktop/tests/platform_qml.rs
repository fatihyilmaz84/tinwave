//! The platform bridge as QML sees it, run headless (`QT_QPA_PLATFORM=offscreen`, software rendering): the real window
//! shell is instantiated by a test-only root (`tests/qml/PlatformProbe.qml`) that prints what `PlatformSettings` and
//! `Theme` say. The OS is only read, never changed; the offscreen window has no native handle, so what a real window
//! gets (Mica, the settings watch) is covered by `platform::windows` tests and the PO runbook.
//!
//! The root-URL override only exists in debug builds, so this file does too.
#![cfg(debug_assertions)]

// Linking the crate also links the Rust half of the CXX-Qt bridge. Only the OS reader is called, to know what the app
// should have read.
use tinwave_desktop::platform;

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
    // The OS is asked read-only, here and in the app: 0 when "animation effects" are off on this machine, else 120.
    // Where the OS cannot be asked the app assumes the defaults, which `unwrap_or_default` gives the same way.
    let os = platform::current().read().unwrap_or_default();
    assert_eq!(
        run.get("durationFast"),
        if os.reduced_motion { "0" } else { "120" },
        "reduced motion is {}",
        os.reduced_motion
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
    // The offscreen plugin's window is not an OS window (its winId is a counter): the bridge does not attach to it
    // and says so, instead of passing a fake handle to the OS and reporting a failed call.
    assert_eq!(run.get("settingsStatus"), "not attached (QPA offscreen)");
    assert!(
        !run.stderr.contains("IsWindow"),
        "no OS call was made for the offscreen window:\n{}",
        run.stderr
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
    assert_eq!(
        run.get("backdropStatus"),
        "solid: not attached (QPA offscreen)"
    );
}

#[test]
fn developer_switches_change_the_app_in_process_only() {
    let run = probe(&["--accent=#d13438", "--transparency=off"]);
    assert_eq!(run.get("accentIsOverride"), "true");
    assert_eq!(run.get("accent").to_lowercase(), "#d13438");
    assert_eq!(run.get("backdropActive"), "false");
    // The offscreen window has no backdrop to turn off, so the status is the "not attached" reason; the switch itself
    // shows in the combined `transparencyEffects` below and in the controller's tests.
    assert_eq!(
        run.get("backdropStatus"),
        "solid: not attached (QPA offscreen)"
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
