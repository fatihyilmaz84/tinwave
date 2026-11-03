use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("Tinwave.Desktop").qml_files([
        QmlFile::from("qml/Theme/Theme.qml").singleton(true),
        QmlFile::from("qml/Main.qml"),
        QmlFile::from("qml/Sidebar.qml"),
    ]))
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .files(["src/app/app_info.rs", "src/app/artwork_bridge.rs"])
    // The default scans and watches the whole crate directory, so every `cargo build` reruns this script
    // (a no-op build then costs ~10 s). Watch nothing extra; the C++ shim files below are tracked individually
    // (ADR 0008).
    .crate_include_root(None)
    // The artwork image provider shim (ADR 0008 decision 4): CXX-Qt cannot subclass QQuickAsyncImageProvider. A header
    // passed to `cpp_file` is run through moc, which `ThumbTask` (Q_OBJECT) needs.
    .include_dir("cpp")
    .cpp_file("cpp/thumb_task.h")
    .cpp_file("cpp/image_provider.cpp")
    .build();
}
