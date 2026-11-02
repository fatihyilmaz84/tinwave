//! Tinwave desktop: Qt 6 Quick/QML window shell over CXX-Qt view models (ADR 0008).
//!
//! The crate is a library plus a thin binary (`main.rs`) so integration tests link the same bridge code the app does.
//! `unsafe` is denied crate-wide; only `#[cxx_qt::bridge]` modules and the documented FFI shim allow it, all in `app`.

pub mod app;
pub mod image;
