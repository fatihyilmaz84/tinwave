//! Application layer: the composition root (`wiring`) and the thin view models QML talks to.
//! The UI never touches SQLite or the engines directly (qml-ui.md); everything reaches it through here.

// `#[cxx_qt::bridge]` expands to `unsafe` / `export_name` items and rejects any other attribute on its module, so
// the allow sits on the `mod` item. Bridge modules are the only place in this crate where it may appear (ADR 0008).
#[allow(unsafe_code)]
pub mod app_info;
// The artwork provider's C++ shim binding and its `extern "C"` callbacks: same rule as above. `ProviderHandle` is an
// opaque C++ type declared in the bridge, which cannot implement `Debug`, and the bridge macro drops the `# Safety`
// doc section of `tw_provider_handle_new` that clippy would look for.
#[allow(unsafe_code, missing_debug_implementations, clippy::missing_safety_doc)]
pub mod artwork_bridge;
// The platform layer's QML object: the raw `QQuickWindow` pointer QML hands to `attachWindow` and the cxx declarations
// of two Qt calls. The OS calls themselves are in `crate::platform::windows`. `QQuickWindow` is an opaque C++ type that
// cannot implement `Debug`, and the bridge macro drops the `# Safety` doc section of the `unsafe fn` invokable that
// clippy would look for.
#[allow(unsafe_code, missing_debug_implementations, clippy::missing_safety_doc)]
pub mod platform_bridge;
pub mod style;
pub mod ui_owner;
pub mod wiring;
