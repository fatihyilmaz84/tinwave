//! The platform bridge as QML sees it, run headless (`QT_QPA_PLATFORM=offscreen`, software rendering): the real window
//! shell is instantiated by a test-only root (`tests/qml/PlatformProbe.qml`) that prints what `PlatformSettings` and
//! `Theme` say. The OS is only read, never changed; the offscreen window has no native handle, so what a real window
//! gets (Mica, the settings watch) is covered by `platform::windows` tests and the PO runbook.
//!
//! The root-URL override only exists in debug builds, so this file does too.
#![cfg(debug_assertions)]
// The `probe` helper is test code that is not itself a `#[test]` function (clippy.toml allows `expect` only inside those).
#![allow(clippy::expect_used)]

// Referenced so the test binary links the crate (and with it the Rust half of the CXX-Qt bridge); it never calls into it.
use tinwave_desktop as _;

mod common;

use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What one probe run printed: the `PROBE name=value` facts and the whole stderr.
struct Run {
    facts: BTreeMap<String, String>,
    stderr: String,
}

impl Run {
    fn get(&self, name: &str) -> &str {
        self.facts
            .get(name)
            .unwrap_or_else(|| panic!("no {name}; stderr was:\n{}", self.stderr))
    }
}

/// Runs the app on the probe root with `args`.
fn probe(args: &[&str]) -> Run {
    let qml = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/qml/PlatformProbe.qml");
    let url = format!("file:///{}", qml.display().to_string().replace('\\', "/"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tinwave-desktop"));
    cmd.args(args)
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QUICK_BACKEND", "software")
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env("TINWAVE_MAIN_QML", url)
        .env_remove("QT_QUICK_CONTROLS_STYLE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    common::add_qt_to_path(&mut cmd);
    let mut child = cmd.spawn().expect("spawn tinwave-desktop");
    let mut pipe = child.stderr.take().expect("piped stderr");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("tinwave-desktop did not exit on its own");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stderr = reader.join().unwrap_or_default();
    assert!(status.success(), "exit {status}; stderr was:\n{stderr}");
    let facts = stderr
        .lines()
        .filter_map(|line| line.split_once("PROBE ")?.1.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        .collect();
    Run { facts, stderr }
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
