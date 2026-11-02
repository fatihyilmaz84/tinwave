use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("Tw.Spike").qml_files([
        QmlFile::from("qml/Theme.qml").singleton(true),
        QmlFile::from("qml/Main.qml"),
        QmlFile::from("qml/Empty.qml"),
        QmlFile::from("qml/TrackRow.qml"),
        QmlFile::from("qml/TwArtwork.qml"),
    ]))
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .crate_include_root(None)
    .include_dir("cpp")
    .files(["src/model.rs", "src/counters.rs", "src/bench.rs", "src/image.rs"])
    .cpp_file("cpp/image_provider.cpp")
    .build();
}
