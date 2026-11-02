//! Composition root (architecture.md: "one composition root, no global singletons elsewhere").
//!
//! Today it only starts Qt: select the controls style, create the QML engine, load `Main.qml` and run the event
//! loop. Engines, stores and view-model dependencies are constructed here and handed to the view models as they
//! arrive with their screen tickets; nothing else in the crate builds a service.

use std::process::ExitCode;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};

use super::style;

/// QML URL of the window shell (resource path of `qml/Main.qml` in the `Tinwave.Desktop` module).
pub const MAIN_QML_URL: &str = "qrc:/qt/qml/Tinwave/Desktop/qml/Main.qml";

/// Developer override for the root QML URL, used by the start-up failure test. Debug builds only: a release build
/// never loads UI from an arbitrary URL.
#[cfg(debug_assertions)]
pub const MAIN_QML_ENV: &str = "TINWAVE_MAIN_QML";

/// Exit code when the root QML object could not be created.
pub const EXIT_QML_LOAD_FAILED: u8 = 2;

/// Exit code when the Qt application object could not be created.
pub const EXIT_QT_INIT_FAILED: u8 = 3;

/// The root QML URL: the override in debug builds, always [`MAIN_QML_URL`] in release builds.
fn main_qml_url() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var(MAIN_QML_ENV) {
        return url;
    }
    MAIN_QML_URL.to_owned()
}

/// Builds the app and runs it to completion. Returns the process exit code.
pub fn run() -> ExitCode {
    let mut app_owner = QGuiApplication::new();
    let Some(mut app) = app_owner.as_mut() else {
        eprintln!("tinwave: could not initialise Qt (no QGuiApplication)");
        return ExitCode::from(EXIT_QT_INIT_FAILED);
    };

    let env_style = std::env::var(style::STYLE_ENV).ok();
    if let Some(name) = style::style_to_select(std::env::consts::OS, env_style.as_deref()) {
        QQuickStyle::set_style(&QString::from(name));
    }

    let mut engine_owner = QQmlApplicationEngine::new();
    let Some(mut engine) = engine_owner.as_mut() else {
        eprintln!("tinwave: could not create the QML engine");
        return ExitCode::from(EXIT_QT_INIT_FAILED);
    };

    // Without this a QML load error leaves `exec()` running forever with no window (ADR 0008, observed in T-0002).
    // Qt has already printed the QML error details through its message handler. Exiting here also covers a root
    // component that finishes loading asynchronously (non-`qrc` override URLs).
    engine
        .as_mut()
        .on_object_creation_failed(|_, url| {
            eprintln!(
                "tinwave: failed to load the user interface from {url}; see the Qt messages above"
            );
            std::process::exit(i32::from(EXIT_QML_LOAD_FAILED));
        })
        .release();

    engine.as_mut().load(&QUrl::from(main_qml_url().as_str()));

    let code = app.as_mut().exec();
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
