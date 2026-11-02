//! `AppInfo`: the first (and smallest) view model. A QML singleton that exposes build facts the shell shows
//! (the version in the app menu). It proves the CXX-Qt wiring end to end; screen view models follow its shape.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, version, READ, CONSTANT)]
        type AppInfo = super::AppInfoRust;
    }
}

use cxx_qt_lib::QString;

/// Rust state of `AppInfo`.
#[derive(Debug)]
pub struct AppInfoRust {
    version: QString,
}

impl Default for AppInfoRust {
    fn default() -> Self {
        Self {
            version: QString::from(env!("CARGO_PKG_VERSION")),
        }
    }
}
