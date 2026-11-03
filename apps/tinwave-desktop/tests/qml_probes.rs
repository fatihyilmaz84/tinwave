//! Two guards that run QML headless (`QT_QPA_PLATFORM=offscreen`, software rendering; no display, GPU or audio device):
//!
//! - the contrast gate of design-system.md 3.1 (`Theme.selfCheck()`: 15 accents x light/dark), so a token edit that breaks
//!   WCAG contrast fails `cargo test` instead of waiting for someone to run the check by hand;
//! - `TwIcon`, which no screen uses yet: a scene with one must load its Lucide SVG from the module's resources without a
//!   single QML warning.
//!
//! Each runs the real binary on a test-only root (`tests/qml/*.qml`, via `TINWAVE_MAIN_QML`), which only exists in debug builds,
//! so this file does too.
#![cfg(debug_assertions)]

// Referenced so the test binary links the crate (and with it the Rust half of the CXX-Qt bridge); it never calls into it.
use tinwave_desktop as _;

mod common;

use common::probe::probe;

#[test]
fn every_check_accent_keeps_every_token_readable_in_both_schemes() {
    let run = probe("ContrastProbe.qml", &[]);
    // The check itself must be intact: the WCAG maths and the full input (15 accents x 2 schemes), or "0 failures" is empty.
    assert_eq!(run.get("blackOnWhite"), "21.00");
    assert_eq!(run.get("whiteOnWhite"), "1.00");
    assert_eq!(run.get("accents"), "15");
    let rows = run.all("row");
    assert_eq!(run.get("rowCount"), "30");
    assert_eq!(rows.len(), 30, "rows: {rows:#?}");

    // Each failing row names the scheme, the accent, the worst text pair ("textFaint on hover/sidebar") and the ratios.
    let failing: Vec<&str> = rows
        .iter()
        .copied()
        .filter(|row| row.ends_with("FAIL"))
        .collect();
    assert!(
        failing.is_empty(),
        "contrast below 4.5:1 (design-system.md 3.1), {} of 30 rows:\n{}",
        failing.len(),
        failing.join("\n")
    );
    assert_eq!(run.get("failures"), "0", "rows: {rows:#?}");
}

#[test]
fn tw_icon_loads_its_svg_and_sizes_itself_without_qml_warnings() {
    let run = probe("IconProbe.qml", &[]);
    assert_eq!(
        run.get("status"),
        "ready",
        "source {}; stderr:\n{}",
        run.get("source"),
        run.stderr
    );
    assert!(
        run.get("source").ends_with("/icons/menu.svg"),
        "source {}",
        run.get("source")
    );
    // Logical size 16 (Theme.iconSizeDense) times the device pixel ratio (1 offscreen) is what the SVG is rasterised at.
    assert_eq!(run.get("implicitWidth"), "16");
    assert_eq!(run.get("implicitHeight"), "16");
    assert_eq!(run.get("sourceWidth"), "16");
    assert_eq!(run.get("sourceHeight"), "16");
    // No name: nothing is loaded, and nothing is complained about.
    assert_eq!(run.get("unnamedSource"), "[]");
    assert_eq!(run.get("unnamedImplicitWidth"), "20");
    // Qt prints every QML warning (binding loops, type errors, a missing resource) with the offending `qrc:` URL. The probe's
    // own `PROBE` lines carry one too (the icon's source), so they are not looked at.
    let problems: Vec<&str> = run
        .stderr
        .lines()
        .filter(|line| line.contains("qrc:") && !line.contains("PROBE "))
        .collect();
    assert!(
        problems.is_empty(),
        "QML reported problems:\n{}",
        problems.join("\n")
    );
}
