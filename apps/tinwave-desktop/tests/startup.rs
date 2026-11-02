//! Start-up behaviour of the real binary, run headless (`QT_QPA_PLATFORM=offscreen`, software rendering): no display,
//! no GPU and no audio device are needed, and no window is ever shown on the desktop.

// Referenced so the test binary links the crate (and with it the Rust half of the CXX-Qt bridge); it never calls into it.
use tinwave_desktop as _;

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Kills the child on drop so a failing assertion never leaves a process behind.
struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tinwave-desktop"));
    cmd.env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QUICK_BACKEND", "software")
        // Qt only writes its messages to a redirected stderr when asked to.
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env_remove("QT_QUICK_CONTROLS_STYLE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Qt is linked dynamically (ADR 0008): its DLLs live next to `qmake`, which the build already needs (`QMAKE`).
    if let Some(qmake) = std::env::var_os("QMAKE").map(PathBuf::from) {
        if let Some(bin) = qmake.parent() {
            let mut paths = vec![bin.to_path_buf()];
            paths.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
    }
    cmd
}

#[cfg(debug_assertions)]
fn read_stderr(child: &mut Child) -> String {
    let mut text = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut text);
    }
    text
}

// The root-URL override only exists in debug builds, so this test does too.
#[cfg(debug_assertions)]
#[test]
fn missing_root_qml_exits_nonzero_with_a_message_instead_of_hanging() {
    let mut child = command()
        .env("TINWAVE_MAIN_QML", "qrc:/tinwave-test/DoesNotExist.qml")
        .spawn()
        .expect("spawn tinwave-desktop");
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("tinwave-desktop did not exit after a QML load failure (it would hang forever)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stderr = read_stderr(&mut child);
    assert_eq!(status.code(), Some(2), "exit code; stderr was:\n{stderr}");
    assert!(
        stderr.contains("failed to load the user interface") && stderr.contains("DoesNotExist.qml"),
        "stderr was:\n{stderr}"
    );
}

#[test]
fn window_shell_loads_without_qml_errors() {
    let mut guard = Guard(command().spawn().expect("spawn tinwave-desktop"));
    // Drain stderr while the app runs: a killed process may lose what it had not yet flushed into the pipe.
    let mut pipe = guard.0.stderr.take().expect("piped stderr");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    });
    // The shell must stay up (the event loop runs); an early exit is a load failure.
    let until = Instant::now() + Duration::from_millis(2500);
    while Instant::now() < until {
        if let Some(status) = guard.0.try_wait().expect("poll child") {
            let stderr = reader.join().unwrap_or_default();
            panic!("shell exited early with {status}; stderr was:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = guard.0.kill();
    let _ = guard.0.wait();
    let stderr = reader.join().unwrap_or_default();
    // Qt prints every QML warning with the offending resource URL (`qrc:/...`); the shell must produce none.
    // (Platform noise such as the offscreen font-directory notice carries no URL.)
    assert!(!stderr.contains("qrc:"), "QML reported problems:\n{stderr}");
}
