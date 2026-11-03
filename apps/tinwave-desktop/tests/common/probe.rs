//! Runs the app binary headless on a test-only QML root (`tests/qml/<name>.qml`, loaded through `TINWAVE_MAIN_QML`) and
//! collects what the root printed as `PROBE name=value` lines. The root-URL override only exists in debug builds, so every
//! test that uses this does too.
// `probe` is test support that is not itself a `#[test]` function (clippy.toml allows `expect` only inside those).
#![allow(clippy::expect_used)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What one probe run printed: the `PROBE name=value` facts (in order; a name may repeat) and the whole stderr.
pub struct Run {
    facts: Vec<(String, String)>,
    pub stderr: String,
}

impl Run {
    /// The first value printed for `name`; panics with the stderr when there is none.
    pub fn get(&self, name: &str) -> &str {
        self.all(name)
            .first()
            .copied()
            .unwrap_or_else(|| panic!("no {name}; stderr was:\n{}", self.stderr))
    }

    /// Every value printed for `name`, in print order.
    pub fn all(&self, name: &str) -> Vec<&str> {
        self.facts
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// Runs the app on the probe root `tests/qml/<qml_file>` with `args`; the process must exit 0 on its own.
pub fn probe(qml_file: &str, args: &[&str]) -> Run {
    let qml = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/qml")
        .join(qml_file);
    let url = format!("file:///{}", qml.display().to_string().replace('\\', "/"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tinwave-desktop"));
    cmd.args(args)
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QUICK_BACKEND", "software")
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env("TINWAVE_MAIN_QML", url)
        .env_remove("QT_QUICK_CONTROLS_STYLE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    super::add_qt_to_path(&mut cmd);
    let mut child = cmd.spawn().expect("spawn tinwave-desktop");
    let mut pipe = child.stderr.take().expect("piped stderr");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("tinwave-desktop did not exit on its own");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stderr = reader.join().unwrap_or_default();
    assert!(status.success(), "exit {status}; stderr was:\n{stderr}");
    let facts = stderr
        .lines()
        .filter_map(|line| line.split_once("PROBE ")?.1.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        .collect();
    Run { facts, stderr }
}
