//! Application layer: the composition root (`wiring`) and the thin view models QML talks to.
//! The UI never touches SQLite or the engines directly (qml-ui.md); everything reaches it through here.

// `#[cxx_qt::bridge]` expands to `unsafe` / `export_name` items and rejects any other attribute on its module, so
// the allow sits on the `mod` item. Bridge modules are the only place in this crate where it may appear (ADR 0008).
#[allow(unsafe_code)]
pub mod app_info;
pub mod style;
pub mod wiring;
