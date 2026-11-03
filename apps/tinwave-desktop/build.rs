// A build script reports every failure by panicking: that is its error channel, so `expect` is the right tool here.
#![allow(clippy::expect_used)]

use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Writes a Qt resource file with every vendored Lucide icon plus the licence text, under the QML module's resource prefix
/// as `icons/<file>`: QML reaches them at `qrc:/qt/qml/Tinwave/Desktop/icons/<name>.svg`. The set is whatever sits in
/// `icons/`; `check_icon_usage` fails the build when it differs from what the QML references, so an unused icon cannot ship.
fn write_icon_qrc() -> PathBuf {
    let dir = Path::new("icons");
    // Cargo reruns this script when the directory listing or any file in it changes.
    println!("cargo::rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read the icons directory")
        .map(|entry| entry.expect("read an icons directory entry").file_name())
        .filter_map(|name| name.into_string().ok())
        .filter(|name| name.ends_with(".svg") || name == "LICENSE-lucide.txt")
        .collect();
    names.sort();
    let abs = std::fs::canonicalize(dir).expect("resolve the icons directory");

    let mut xml = String::from("<RCC>\n  <qresource prefix=\"/qt/qml/Tinwave/Desktop\">\n");
    for name in &names {
        let path = abs.join(name);
        writeln!(
            xml,
            "    <file alias=\"icons/{name}\">{}</file>",
            path.display()
        )
        .expect("write to a String");
    }
    xml.push_str("  </qresource>\n</RCC>\n");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"))
        .join("icons.qrc");
    std::fs::write(&out, xml).expect("write icons.qrc");
    out
}

/// Names of the vendored icons: the `*.svg` files in `icons/`.
fn vendored_icons(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("read the icons directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|file| file.strip_suffix(".svg").map(str::to_owned))
        .collect()
}

/// Icon names QML refers to. QML writes them only in three forms, always as string literals: `iconName: "x"` (model roles
/// and properties), `TwIcons.source("x")` and `name: "x"` inside a `TwIcon { ... }` block.
fn referenced_icons(qml_dir: &Path, out: &mut BTreeSet<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(qml_dir)
        .expect("read the qml directory")
        .map(|entry| entry.expect("read a qml directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            referenced_icons(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "qml") {
            let text = std::fs::read_to_string(&path).expect("read a QML file");
            literals_after(&text, "iconName: \"", out);
            literals_after(&text, "TwIcons.source(\"", out);
            // `name: "x"` is only an icon name inside a `TwIcon { ... }` block (not `Accessible.name:`).
            for (start, _) in text.match_indices("TwIcon {") {
                let block = text[start..].split('}').next().unwrap_or_default();
                for (at, _) in block.match_indices("name: \"") {
                    if !block[..at].ends_with(|c: char| c == '.' || c == '_' || c.is_alphanumeric())
                    {
                        literals_after(&block[at..], "name: \"", out);
                    }
                }
            }
        }
    }
}

/// Collects the string literal that follows every occurrence of `prefix` (which ends in the opening quote).
fn literals_after(text: &str, prefix: &str, out: &mut BTreeSet<String>) {
    for (at, _) in text.match_indices(prefix) {
        let rest = &text[at + prefix.len()..];
        if let Some(end) = rest.find('"') {
            out.insert(rest[..end].to_owned());
        }
    }
}

/// Unused icons are not shipped (design-system.md 3.4): every vendored icon must be referenced from QML, and every icon QML
/// names must exist. Runs on every build, so the vendored set is always exactly the used set.
fn check_icon_usage() {
    let vendored = vendored_icons(Path::new("icons"));
    let mut referenced = BTreeSet::new();
    referenced_icons(Path::new("qml"), &mut referenced);
    let unused: Vec<_> = vendored.difference(&referenced).collect();
    let missing: Vec<_> = referenced.difference(&vendored).collect();
    assert!(
        unused.is_empty() && missing.is_empty(),
        "icons/ and the QML disagree. Vendored but not used in QML (delete the file): {unused:?}. \
         Used in QML but not vendored (run tools/vendor-icons.ps1 -Add <name>): {missing:?}"
    );
}

fn main() {
    check_icon_usage();
    CxxQtBuilder::new_qml_module(QmlModule::new("Tinwave.Desktop").qml_files([
        QmlFile::from("qml/Theme/Theme.qml").singleton(true),
        QmlFile::from("qml/Tw/TwIcons.qml").singleton(true),
        QmlFile::from("qml/Tw/TwIcon.qml"),
        QmlFile::from("qml/Main.qml"),
        QmlFile::from("qml/Sidebar.qml"),
    ]))
    .qrc(write_icon_qrc())
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
