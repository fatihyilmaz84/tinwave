// A build script reports every failure by panicking: that is its error channel, so `expect` is the right tool here.
#![allow(clippy::expect_used)]

use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlFile, QmlModule};
use std::collections::BTreeSet;
use std::path::Path;

/// File names in `icons/` that pass `keep`, sorted.
fn icon_files(keep: impl Fn(&str) -> bool) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir("icons")
        .expect("read the icons directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| keep(name))
        .collect();
    names.sort();
    names
}

/// Every vendored Lucide icon plus the licence text, as resources `icons/<file>` of the QML module (`CxxQtBuilder`
/// prefixes them with `/qt/qml/Tinwave/Desktop`): QML reaches them at `qrc:/qt/qml/Tinwave/Desktop/icons/<name>.svg`.
/// The set is whatever sits in `icons/`; `check_icon_usage` fails the build when it differs from what the QML references,
/// so an unused icon cannot ship. Cargo watches each file (and the directory, for added or removed ones).
fn icon_resources() -> QResources {
    println!("cargo::rerun-if-changed=icons");
    let files = icon_files(|name| name.ends_with(".svg") || name == "LICENSE-lucide.txt");
    // The resource path is the path as written (forward slashes); `CxxQtBuilder` resolves the file itself.
    QResources::new()
        .resource(QResource::new().files(files.iter().map(|name| format!("icons/{name}"))))
}

/// Removes every `//` and `/* */` comment from QML source (string literals are left alone).
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            while chars.peek().is_some_and(|&n| n != '\n') {
                chars.next();
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(n) = chars.next() {
                if n == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn is_identifier_char(c: char) -> bool {
    c == '_' || c == '.' || c.is_alphanumeric()
}

/// Collects the string literal in `key <ws> sep <ws> "literal"` for every occurrence of `key` that starts a name (not
/// preceded by an identifier character or `.`, so `Accessible.name:` is not `name:`).
fn literals_after(text: &str, key: &str, sep: char, out: &mut BTreeSet<String>) {
    for (at, _) in text.match_indices(key) {
        if text[..at].ends_with(is_identifier_char) {
            continue;
        }
        let rest = text[at + key.len()..].trim_start();
        let Some(rest) = rest.strip_prefix(sep) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            out.insert(rest[..end].to_owned());
        }
    }
}

/// The text between the braces of every `TwIcon { ... }` (nested braces included).
fn twicon_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    for (at, _) in text.match_indices("TwIcon") {
        let after = &text[at + "TwIcon".len()..];
        if text[..at].ends_with(is_identifier_char) || after.starts_with(is_identifier_char) {
            continue; // part of another name, e.g. TwIcons
        }
        let Some(body) = after.trim_start().strip_prefix('{') else {
            continue;
        };
        let mut depth = 1;
        for (i, c) in body.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                blocks.push(&body[..i]);
                break;
            }
        }
    }
    blocks
}

/// Icon names QML refers to. QML writes them only in three forms, always as string literals (whitespace and comments do not
/// matter): `iconName: "x"` (model roles and properties), `TwIcons.source("x")` and `name: "x"` inside a `TwIcon { ... }`.
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
            let text = strip_comments(&std::fs::read_to_string(&path).expect("read a QML file"));
            literals_after(&text, "iconName", ':', out);
            literals_after(&text, "TwIcons.source", '(', out);
            for block in twicon_blocks(&text) {
                literals_after(block, "name", ':', out);
            }
        }
    }
}

/// Unused icons are not shipped (design-system.md 3.4): every vendored icon must be referenced from QML, and every icon QML
/// names must exist. Runs on every build, so the vendored set is always exactly the used set.
fn check_icon_usage() {
    let vendored: BTreeSet<String> = icon_files(|name| name.ends_with(".svg"))
        .into_iter()
        .filter_map(|file| file.strip_suffix(".svg").map(str::to_owned))
        .collect();
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
    .qrc_resources(icon_resources())
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .files([
        "src/app/app_info.rs",
        "src/app/artwork_bridge.rs",
        "src/app/platform_bridge.rs",
    ])
    // The default scans and watches the whole crate directory, so every `cargo build` reruns this script
    // (a no-op build then costs ~10 s). Watch nothing extra; the C++ shim files below are tracked individually
    // (ADR 0008).
    .crate_include_root(None)
    // The artwork image provider shim (ADR 0008 decision 4): CXX-Qt cannot subclass QQuickAsyncImageProvider. A header
    // passed to `cpp_file` is run through moc, which `ThumbTask` (Q_OBJECT) needs.
    .include_dir("cpp")
    .cpp_file("cpp/thumb_task.h")
    .cpp_file("cpp/image_provider.cpp")
    // winId() and the platform plugin name as rust::usize / rust::String (see the header for why).
    .cpp_file("cpp/platform_window.cpp")
    .build();
}
