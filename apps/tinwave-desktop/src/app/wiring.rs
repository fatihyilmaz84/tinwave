//! Composition root (architecture.md: "one composition root, no global singletons elsewhere").
//!
//! Today it starts Qt: select the controls style, create the QML engine, install the artwork image provider, load
//! `Main.qml` and run the event loop. Engines, stores and view-model dependencies are constructed here and handed to
//! the view models as they arrive with their screen tickets; nothing else in the crate builds a service.
//!
//! The engine and everything tied to its lifetime live in a [`UiOwner`] inside [`Shell`]; see there for the teardown
//! order that keeps the image provider from crashing Qt (ADR 0008).

use std::pin::Pin;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQmlEngine, QQuickStyle, QString, QUrl};

use super::artwork_bridge::ArtworkProvider;
use super::style;
use super::ui_owner::UiOwner;
use crate::image::{self, ArtworkSource, NoArtwork};

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

/// What [`Shell::start`] builds. [`ShellConfig::production`] is what the app runs; the stress tool and the tests vary
/// the pieces they exercise.
pub struct ShellConfig {
    /// Root QML URL to load.
    pub qml_url: String,
    /// Where artwork thumbnails come from.
    pub artwork: Arc<dyn ArtworkSource>,
    /// Worker threads of the artwork provider.
    pub artwork_pool_threads: i32,
    /// Whether to select the platform controls style (the stress tool does not need one).
    pub select_style: bool,
}

impl std::fmt::Debug for ShellConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellConfig")
            .field("qml_url", &self.qml_url)
            .field("artwork_pool_threads", &self.artwork_pool_threads)
            .field("select_style", &self.select_style)
            .finish_non_exhaustive()
    }
}

impl ShellConfig {
    /// The app's own configuration: `Main.qml`, no artwork source until the library provides one, platform style.
    pub fn production() -> Self {
        Self {
            qml_url: main_qml_url(),
            artwork: Arc::new(NoArtwork),
            artwork_pool_threads: image::POOL_THREADS,
            select_style: true,
        }
    }
}

/// The running UI: the Qt application and the [`UiOwner`] (engine + image provider).
///
/// Field order is the drop order: the owner (quiesce, then engine) goes before the artwork handle, which goes
/// before the application object.
pub struct Shell {
    ui: UiOwner<UniquePtr<QQmlApplicationEngine>>,
    artwork: Rc<ArtworkProvider>,
    app: UniquePtr<QGuiApplication>,
    load_failed: Arc<AtomicBool>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("ui", &self.ui)
            .finish_non_exhaustive()
    }
}

impl Shell {
    /// Creates the application, the engine and the provider, and starts loading the root QML. On failure returns the
    /// process exit code to use.
    pub fn start(config: ShellConfig) -> Result<Self, ExitCode> {
        let app = QGuiApplication::new();
        if app.is_null() {
            eprintln!("tinwave: could not initialise Qt (no QGuiApplication)");
            return Err(ExitCode::from(EXIT_QT_INIT_FAILED));
        }

        if config.select_style {
            let env_style = std::env::var(style::STYLE_ENV).ok();
            if let Some(name) = style::style_to_select(std::env::consts::OS, env_style.as_deref()) {
                QQuickStyle::set_style(&QString::from(name));
            }
        }

        let mut ui = UiOwner::new(QQmlApplicationEngine::new());

        // The engine takes the provider object; the handle keeps its shared state so the owner can stop the workers
        // before the engine is dropped. It is registered with the owner right away, before anything can load or fail.
        let artwork = Rc::new(ArtworkProvider::install(
            engine_of(&mut ui)?.as_mut(),
            image::PROVIDER_NAME,
            config.artwork,
            config.artwork_pool_threads,
        ));
        ui.add_provider(Rc::<ArtworkProvider>::clone(&artwork));

        // Without this a QML load error leaves `exec()` running forever with no window (ADR 0008, observed in
        // T-0002). Qt has already printed the QML error details through its message handler. The failure is recorded
        // for `exec`, which then reports exit code 2 after the normal teardown (providers quiesced, engine dropped);
        // nothing calls `process::exit`, which would skip both. A root component that finishes loading
        // asynchronously (non-`qrc` override URLs) fails while the loop runs, so the handler also asks the engine to
        // exit it. Before `exec` starts the engine's exit request has no effect, hence the flag.
        let load_failed = Arc::new(AtomicBool::new(false));
        let failed = Arc::clone(&load_failed);
        let mut engine = engine_of(&mut ui)?;
        engine
            .as_mut()
            .on_object_creation_failed(move |engine, url| {
                eprintln!(
                    "tinwave: failed to load the user interface from {url}; see the Qt messages above"
                );
                failed.store(true, Ordering::SeqCst);
                let engine: Pin<&mut QQmlEngine> = engine.upcast_pin();
                engine.exit(i32::from(EXIT_QML_LOAD_FAILED));
            })
            .release();

        engine.as_mut().load(&QUrl::from(config.qml_url.as_str()));

        Ok(Self {
            ui,
            artwork,
            app,
            load_failed,
        })
    }

    /// Runs the event loop until the application quits. Returns Qt's exit code, or [`EXIT_QML_LOAD_FAILED`] when the
    /// root QML could not be loaded (then the loop is not even entered if the failure was already known).
    pub fn exec(&mut self) -> i32 {
        let load_failed = || self.load_failed.load(Ordering::SeqCst);
        if load_failed() {
            return i32::from(EXIT_QML_LOAD_FAILED);
        }
        let code = match self.app.as_mut() {
            Some(app) => app.exec(),
            None => i32::from(EXIT_QT_INIT_FAILED),
        };
        if load_failed() {
            i32::from(EXIT_QML_LOAD_FAILED)
        } else {
            code
        }
    }

    /// Stops the artwork workers. Call it right after [`exec`](Self::exec) returned; dropping the shell repeats it.
    pub fn shutdown(&mut self) {
        self.ui.shutdown();
    }

    /// The artwork provider handle (counters).
    pub fn artwork(&self) -> &ArtworkProvider {
        &self.artwork
    }
}

/// The owner's engine, or the exit code to use when Qt could not create one.
fn engine_of(
    ui: &mut UiOwner<UniquePtr<QQmlApplicationEngine>>,
) -> Result<Pin<&mut QQmlApplicationEngine>, ExitCode> {
    ui.engine_mut().as_mut().ok_or_else(|| {
        eprintln!("tinwave: could not create the QML engine");
        ExitCode::from(EXIT_QT_INIT_FAILED)
    })
}

/// Builds the app and runs it to completion. Returns the process exit code.
pub fn run() -> ExitCode {
    let mut shell = match Shell::start(ShellConfig::production()) {
        Ok(shell) => shell,
        Err(code) => return code,
    };
    let code = shell.exec();
    // Explicit, not left to Drop: the workers must be stopped while the engine and Qt's reader thread still exist.
    shell.shutdown();
    drop(shell);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
