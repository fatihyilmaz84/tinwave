use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("Tinwave.Desktop").qml_files([
        QmlFile::from("qml/Theme/Theme.qml").singleton(true),
        QmlFile::from("qml/Main.qml"),
        QmlFile::from("qml/Sidebar.qml"),
    ]))
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .files(["src/app/app_info.rs"])
    // The default scans and watches the whole crate directory, so every `cargo build` reruns this script
    // (a no-op build then costs ~10 s). We have no C++ headers of our own: watch nothing extra (ADR 0008).
    .crate_include_root(None)
    .build();
}
