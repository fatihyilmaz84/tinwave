//! Helpers shared by the integration tests that run the app's binaries headless.

use std::path::PathBuf;
use std::process::Command;

/// Puts the Qt `bin` folder first on the child's `PATH`. Qt is linked dynamically (ADR 0008): its DLLs live next to
/// `qmake`, which the build already needs (`QMAKE`).
pub fn add_qt_to_path(cmd: &mut Command) {
    if let Some(qmake) = std::env::var_os("QMAKE").map(PathBuf::from) {
        if let Some(bin) = qmake.parent() {
            let mut paths = vec![bin.to_path_buf()];
            paths.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
    }
}
