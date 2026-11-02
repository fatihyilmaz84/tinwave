//! T-0002 spike: CXX-Qt + QML virtualized list. Throwaway prototype.

mod bench;
mod counters;
mod image;
mod metrics;
mod model;

use std::pin::Pin;

use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQmlEngine, QQuickStyle, QString, QUrl};

fn main() {
    metrics::init();
    metrics::install_crash_handler();
    let mut app = QGuiApplication::new();
    println!("T2 app_created since_main_ms={:.1}", metrics::since_main_ms());

    // Design system: FluentWinUI3 on Windows (ADR 0003). Overridable for T-0011-style checks.
    let style = std::env::var("T2_STYLE").unwrap_or_else(|_| "FluentWinUI3".to_string());
    QQuickStyle::set_style(&QString::from(style.as_str()));

    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        let qml_engine: Pin<&mut QQmlEngine> = engine.upcast_pin();
        // SAFETY: ownership of the provider moves to the QML engine.
        unsafe {
            qml_engine.add_image_provider(&QString::from("art"), image::tw_make_image_provider())
        };
    }

    // A QML load error otherwise leaves exec() running forever with no window (observed: process hung 120 s).
    if let Some(engine) = engine.as_mut() {
        engine
            .on_object_creation_failed(|_, url| {
                eprintln!("QML load failed: {url}");
                std::process::exit(2);
            })
            .release();
    }

    let main_qml = match std::env::var("T2_MODE").as_deref() {
        Ok("empty") => "qrc:/qt/qml/Tw/Spike/qml/Empty.qml",
        Ok("stress") => "qrc:/qt/qml/Tw/Spike/qml/Stress.qml", // T-0041 teardown/cancel storm
        Ok("broken") => "qrc:/qt/qml/Tw/Spike/qml/DoesNotExist.qml", // failure-path check
        _ => "qrc:/qt/qml/Tw/Spike/qml/Main.qml",
    };
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(main_qml));
    }
    println!("T2 engine_load_returned since_main_ms={:.1}", metrics::since_main_ms());
    if let Some(app) = app.as_mut() {
        app.exec();
    }
    // T-0041: stop the image worker threads while the QML engine (and Qt's pixmap-reader thread) still exist.
    image::tw_image_provider_shutdown();
    drop(engine);
}
