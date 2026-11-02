//! The artwork image provider end to end, headless (`QT_QPA_PLATFORM=offscreen`, software rendering): the dev-only
//! `image-stress` binary runs the app's own startup and teardown (`Shell`: engine + provider + UI owner, explicit
//! shutdown after `exec()`) around a stress scene and reports the provider's counters.
//!
//! Needs no display, no GPU and no audio device. The Release stress run (>= 300 runs) is
//! `tools/stress/run-stress.ps1`; these tests are its small, deterministic cousins.

// Referenced so the test binary links the crate (and with it the Rust half of the CXX-Qt bridge); it never calls into it.
use tinwave_desktop as _;

mod common;

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A run longer than this is a hang at exit (a healthy run takes one or two seconds even in a debug build).
const HANG_AFTER: Duration = Duration::from_secs(60);

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    /// The `key=value` counters of the `STRESS_DONE` line.
    fn counters(&self) -> HashMap<String, u64> {
        let line = self
            .stdout
            .lines()
            .find(|l| l.starts_with("STRESS_DONE"))
            .unwrap_or_else(|| self.fail("no STRESS_DONE line"));
        line.split_whitespace()
            .skip(1)
            .filter_map(|kv| kv.split_once('='))
            .filter_map(|(k, v)| Some((k.to_owned(), v.parse().ok()?)))
            .collect()
    }

    fn fail(&self, what: &str) -> ! {
        panic!(
            "{what}; exit code {:?}\n--- stdout\n{}\n--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }

    /// Asserts the run exited 0 and went through the whole teardown, then returns its counters.
    fn expect_clean_exit(&self) -> HashMap<String, u64> {
        if self.code != Some(0) {
            self.fail("did not exit with code 0");
        }
        if !self.stdout.contains("STRESS_TEARDOWN_COMPLETE") {
            self.fail("the engine teardown did not complete");
        }
        let counters = self.counters();
        assert_eq!(
            counters["duplicate_finishes"], 0,
            "a second finished() was attempted"
        );
        counters
    }
}

fn scene(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tools/stress")
        .join(name)
        .display()
        .to_string()
}

fn run_stress(args: &[String]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_image-stress"));
    cmd.args(args)
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QUICK_BACKEND", "software")
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    common::add_qt_to_path(&mut cmd);
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("spawn image-stress: {e}"));
    let mut out = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("no piped stdout"));
    let mut err = child
        .stderr
        .take()
        .unwrap_or_else(|| panic!("no piped stderr"));
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + HANG_AFTER;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .unwrap_or_else(|e| panic!("poll child: {e}"))
        {
            break Some(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let run = Run {
        code: status.and_then(|s| s.code()),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    };
    assert!(
        status.is_some(),
        "image-stress did not exit within {HANG_AFTER:?} (hang at exit)\n--- stdout\n{}\n--- stderr\n{}",
        run.stdout,
        run.stderr
    );
    run
}

#[test]
fn every_response_gets_exactly_one_finished_and_the_cache_serves_repeats() {
    let run = run_stress(&[format!("--qml={}", scene("Settle.qml"))]);
    let c = run.expect_clean_exit();
    // 64 images, all settled before quitting: each response was completed once (ready or error), none twice.
    assert_eq!(c["responses"], 64, "{c:?}");
    assert_eq!(c["finished"], 64, "{c:?}");
    assert_eq!(c["duplicate_finishes"], 0);
    assert_eq!(c["tasks_skipped"], 0, "{c:?}");
    // 14 distinct artwork keys + 8 keys without artwork (never cached, asked again): with one worker every repeat of
    // a key finds the first result in the cache, so the source ran 14 + 8 times for 64 requests.
    assert_eq!(c["requests"], 64, "{c:?}");
    assert_eq!(c["source_calls"], 22, "{c:?}");
    assert_eq!(c["cache_hits"], 42, "{c:?}");
    assert_eq!(c["misses"], 8, "{c:?}");
}

#[test]
fn cancel_storm_with_requests_in_flight_then_quit_exits_cleanly() {
    // (mode, ms until Qt.quit()): the quit lands while hundreds of requests are queued, running or being cancelled.
    let cases = [
        ("storm", 150),
        ("storm", 300),
        ("storm", 500),
        ("storm", 800),
        ("scroll", 200),
        ("scroll", 450),
        ("scroll", 700),
    ];
    let mut tasks_run = 0;
    for (mode, exit_ms) in cases {
        let run = run_stress(&[
            format!("--qml={}", scene("Storm.qml")),
            format!("--mode={mode}"),
            format!("--exit-ms={exit_ms}"),
            "--pool=1".to_owned(),
        ]);
        let c = run.expect_clean_exit();
        assert!(
            c["responses"] > 0,
            "{mode} {exit_ms} ms: no requests at all: {c:?}"
        );
        assert!(
            c["cancels"] > 0,
            "{mode} {exit_ms} ms: nothing was cancelled: {c:?}"
        );
        assert!(
            c["finished"] <= c["responses"],
            "{mode} {exit_ms} ms: more finished() than responses: {c:?}"
        );
        tasks_run += c["tasks_run"];
    }
    assert!(
        tasks_run > 0,
        "the workers never ran a task, so nothing was in flight"
    );
}
