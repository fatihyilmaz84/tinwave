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

/// Runs `Storm.qml` once in `mode`, waiting `exit_ms` after the provider's first answer, and checks what every such run
/// must show, however loaded the machine: a clean exit and consistent counters. Whether the run also quit with
/// requests in flight and cancellations done is up to [`quit_in_flight`].
fn storm_run(mode: &str, exit_ms: u32) -> HashMap<String, u64> {
    let run = run_stress(&[
        format!("--qml={}", scene("Storm.qml")),
        format!("--mode={mode}"),
        format!("--exit-ms={exit_ms}"),
        "--pool=1".to_owned(),
    ]);
    let c = run.expect_clean_exit();
    assert!(
        c["finished"] <= c["responses"],
        "{mode} {exit_ms} ms: more finished() than responses: {c:?}"
    );
    c
}

/// Whether a storm run quit with requests still in flight after cancellations had happened: the only runs that say
/// anything about quitting in the middle of a cancel storm. "In flight" is `in_flight_at_quit`, read at the instant the
/// event loop ended (before the quiesce), not derived from the final counters: once the providers are quiesced the
/// queued tasks drain within microseconds, and how many of them the reader thread has already delivered by the time the
/// counters are read depends on CPU load (T-0049: this test failed 2 of 14 runs under a loaded full-workspace run when
/// it compared the final counters).
fn quit_in_flight(c: &HashMap<String, u64>) -> bool {
    c["in_flight_at_quit"] > 0 && c["cancels"] > 0
}

#[test]
fn cancel_storm_with_requests_in_flight_then_quit_exits_cleanly() {
    // What this proves: quitting while requests are queued, running or being cancelled tears down cleanly (exit 0, the
    // engine is destroyed, no second finished()). It only proves that if a run really quit with requests in flight and
    // after cancellations, so each mode must produce at least one such run.
    //
    // Every run must be clean; only the "really was in flight" part is a precondition that load can spoil, so it is
    // checked per mode over all runs, not per run. A run's exit time is not fixed (T-0071: a fixed 150 ms from scene
    // load produced "no requests at all" and "nothing was cancelled" when the machine was too loaded to run the
    // provider's reader thread in time): `Storm.qml` starts its exit timer only after the provider answered a first
    // request and quits only after the grid jumped a few times since. That makes "no requests at all" impossible, and
    // "nothing was cancelled" a matter of the reader thread lagging behind the jumps; if load still spoils a mode's
    // runs, it gets a few more attempts.
    //
    // (mode, ms to wait after the first answer): the quit lands while hundreds of requests are queued, running or
    // being cancelled.
    let cases = [
        ("storm", 150),
        ("storm", 300),
        ("storm", 500),
        ("storm", 800),
        ("scroll", 200),
        ("scroll", 450),
        ("scroll", 700),
    ];
    // Attempts after the listed cases, spent on whatever is still missing (bounded, never open-ended) before failing.
    const EXTRA_ATTEMPTS: u32 = 6;

    #[derive(Default)]
    struct Covered {
        storm: u32,
        scroll: u32,
        tasks_run: u64,
    }
    impl Covered {
        fn add(&mut self, mode: &str, c: &HashMap<String, u64>) {
            self.tasks_run += c["tasks_run"];
            if quit_in_flight(c) {
                match mode {
                    "storm" => self.storm += 1,
                    _ => self.scroll += 1,
                }
            }
        }
    }

    let mut covered = Covered::default();
    for (mode, exit_ms) in cases {
        covered.add(mode, &storm_run(mode, exit_ms));
    }
    for _ in 0..EXTRA_ATTEMPTS {
        // A scroll run is also the likeliest to have a worker run a task.
        let (mode, exit_ms) = if covered.storm == 0 {
            ("storm", 500)
        } else if covered.scroll == 0 || covered.tasks_run == 0 {
            ("scroll", 450)
        } else {
            break;
        };
        covered.add(mode, &storm_run(mode, exit_ms));
    }
    assert!(
        covered.storm > 0 && covered.scroll > 0,
        "no run quit with requests in flight after cancellations (storm {}, scroll {}) even after {EXTRA_ATTEMPTS} extra attempts",
        covered.storm,
        covered.scroll
    );
    assert!(
        covered.tasks_run > 0,
        "the workers never ran a task, even after {EXTRA_ATTEMPTS} extra attempts, so no task was running at a quit"
    );
}

/// One `SIZE` line of `Sizes.qml`.
#[derive(Debug, PartialEq, Eq)]
struct Shown {
    ready: bool,
    width: u32,
    height: u32,
}

/// Parses `index label status=Ready|Error w=<n> h=<n>` (the text after `SIZE `).
fn parse_size_line(rest: &str) -> Option<(usize, Shown)> {
    let mut parts = rest.split_whitespace();
    let index: usize = parts.next()?.parse().ok()?;
    let _label = parts.next()?;
    let mut field = |name: &str| -> Option<String> {
        parts
            .next()?
            .strip_prefix(name)?
            .strip_prefix('=')
            .map(str::to_owned)
    };
    let ready = field("status")? == "Ready";
    let width = field("w")?.parse().ok()?;
    let height = field("h")?.parse().ok()?;
    Some((
        index,
        Shown {
            ready,
            width,
            height,
        },
    ))
}

/// Runs `Sizes.qml` with `images` (`key@WxH,...`) and the extra stress arguments; returns one entry per image, in
/// order, plus the run (for its counters and the QML warnings in stderr).
fn sizes_run(images: &str, extra: &[&str]) -> (Vec<Shown>, Run) {
    let mut args = vec![
        format!("--qml={}", scene("Sizes.qml")),
        format!("--images={images}"),
    ];
    args.extend(extra.iter().map(|a| (*a).to_owned()));
    let run = run_stress(&args);
    run.expect_clean_exit();
    let log = format!("{}\n{}", run.stdout, run.stderr);
    assert!(
        log.contains("SIZES_DONE"),
        "the scene did not settle\n{log}"
    );
    let mut shown: Vec<(usize, Shown)> = log
        .lines()
        .filter_map(|l| parse_size_line(l.split_once("SIZE ")?.1))
        .collect();
    shown.sort_by_key(|(index, _)| *index);
    let count = images.split(',').filter(|e| !e.is_empty()).count();
    assert_eq!(shown.len(), count, "one SIZE line per image\n{log}");
    (shown.into_iter().map(|(_, s)| s).collect(), run)
}

fn ready(width: u32, height: u32) -> Shown {
    Shown {
        ready: true,
        width,
        height,
    }
}

#[test]
fn refused_and_missing_artwork_are_image_error_and_served_sizes_are_image_ready() {
    let (shown, run) = sizes_run(
        "a@48x48,b@1024x8,c@2000x2000,d@1025x10,e@10x1025,missing-1@48x48",
        &[],
    );
    let c = run.counters();
    // In range (the limit itself included) is served at exactly the size asked for.
    assert_eq!(shown[0], ready(48, 48), "{shown:?}");
    assert_eq!(shown[1], ready(1024, 8), "{shown:?}");
    // Above the limit in either dimension, and a key without artwork: Image.Error.
    for (i, what) in [
        (2, "2000x2000"),
        (3, "1025x10"),
        (4, "10x1025"),
        (5, "no artwork"),
    ] {
        assert!(!shown[i].ready, "{what} must be Image.Error: {shown:?}");
    }
    // Every response completed once; the refused ones ran no task and asked Rust for nothing.
    assert_eq!((c["responses"], c["finished"]), (6, 6), "{c:?}");
    assert_eq!(c["tasks_run"], 3, "{c:?}");
    assert_eq!(c["requests"], 3, "{c:?}");
    assert_eq!(c["misses"], 1, "{c:?}");
    // Image.Error carries a message (Qt logs it as a QML warning).
    let log = &run.stderr;
    assert!(
        log.contains("2000x2000 is above the limit of 1024 px per edge"),
        "{log}"
    );
    assert!(log.contains("no artwork for \"missing-1\""), "{log}");
}

#[test]
fn the_edge_limit_comes_from_rust_not_from_a_second_literal_in_the_shim() {
    let images = "a@48x48,b@32x32,c@1500x8";
    // Lowered to 32: 48 is refused, 32 still served.
    let (low, run) = sizes_run(images, &["--max-edge=32"]);
    assert!(!low[0].ready, "48 above a limit of 32: {low:?}");
    assert_eq!(low[1], ready(32, 32), "{low:?}");
    assert!(!low[2].ready, "{low:?}");
    assert!(
        run.stderr.contains("48x48 is above the limit of 32 px"),
        "{}",
        run.stderr
    );
    assert_eq!(run.counters()["tasks_run"], 1);
    // Default: 1500 is above the production limit (1024).
    let (default, _) = sizes_run(images, &[]);
    assert!(default[0].ready && default[1].ready, "{default:?}");
    assert!(!default[2].ready, "{default:?}");
    // Raised to 2048: the same request is served. Only the Rust-side value changed.
    let (high, run) = sizes_run(images, &["--max-edge=2048"]);
    assert_eq!(high[2], ready(1500, 8), "{high:?}");
    assert_eq!(run.counters()["tasks_run"], 3);
}

#[test]
fn a_one_edge_source_size_keeps_that_edge() {
    // 64x0 and 0x64 (the other dimension unset): a 64 px square, the source's aspect being unknown; neither set: the
    // 128 px default; both set: as asked.
    let (shown, run) = sizes_run("a@64x0,b@0x64,c@0x0,d@64x32", &[]);
    assert_eq!(shown[0], ready(64, 64), "{shown:?}");
    assert_eq!(shown[1], ready(64, 64), "{shown:?}");
    assert_eq!(shown[2], ready(128, 128), "{shown:?}");
    assert_eq!(shown[3], ready(64, 32), "{shown:?}");
    assert_eq!(run.counters()["tasks_run"], 4);
}
