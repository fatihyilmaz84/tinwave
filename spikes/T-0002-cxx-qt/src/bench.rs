//! Measurement helper exposed to QML (spike only): first-frame time, per-phase frame stats, RSS, CPU.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type Bench = super::BenchRust;

        #[qinvokable]
        fn env(self: &Bench, name: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "firstFrame"]
        fn first_frame(self: &Bench);

        #[qinvokable]
        fn phase(self: &Bench, name: &QString);

        #[qinvokable]
        #[cxx_name = "frameSwapped"]
        fn frame_swapped(self: &Bench);

        #[qinvokable]
        fn log(self: &Bench, msg: &QString);

        #[qinvokable]
        fn report(self: &Bench);
    }
}

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::metrics;
use cxx_qt_lib::QString;

#[derive(Default)]
struct Phase {
    wall_start: Option<Instant>,
    wall_ms: f64,
    cpu_start: f64,
    cpu_ms: f64,
    deltas_ms: Vec<f32>,
    frames: u64,
    peak_ws: u64,
    peak_private: u64,
    ws_start: u64,
    ws_end: u64,
}

#[derive(Default)]
struct State {
    order: Vec<String>,
    phases: HashMap<String, Phase>,
    current: Option<String>,
    last_swap: Option<Instant>,
    first_done: bool,
    frame_counter: u64,
}

/// Rust state of `Bench`.
#[derive(Default)]
pub struct BenchRust {
    state: Mutex<State>,
}

fn close(p: &mut Phase) {
    if let Some(s) = p.wall_start.take() {
        p.wall_ms = s.elapsed().as_secs_f64() * 1000.0;
        p.cpu_ms = metrics::cpu_ms() - p.cpu_start;
        p.ws_end = metrics::mem().working_set;
    }
}

impl qobject::Bench {
    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn env(&self, name: &QString) -> QString {
        QString::from(std::env::var(name.to_string()).unwrap_or_default().as_str())
    }

    pub fn first_frame(&self) {
        let mut s = self.st();
        if s.first_done {
            return;
        }
        s.first_done = true;
        let m = metrics::mem();
        println!(
            "T2 first_frame since_main_ms={:.1} process_age_ms={:.1} ws_mb={:.1} pws_mb={:.1} cpu_ms={:.0}",
            metrics::since_main_ms(),
            metrics::process_age_ms(),
            metrics::mb(m.working_set),
            metrics::mb(m.private_ws),
            metrics::cpu_ms()
        );
        if std::env::var_os("T2_EXIT_AT_FIRST_FRAME").is_some() {
            std::process::exit(0); // spike-only: startup timing runs
        }
    }

    pub fn phase(&self, name: &QString) {
        let name = name.to_string();
        let mut s = self.st();
        if let Some(cur) = s.current.take() {
            if let Some(p) = s.phases.get_mut(&cur) {
                close(p);
            }
        }
        let m = metrics::mem();
        if !s.phases.contains_key(&name) {
            s.order.push(name.clone());
        }
        let p = s.phases.entry(name.clone()).or_default();
        p.wall_start = Some(Instant::now());
        p.cpu_start = metrics::cpu_ms();
        p.ws_start = m.working_set;
        p.peak_ws = m.working_set;
        p.peak_private = m.private_ws;
        s.current = Some(name);
        s.last_swap = None;
    }

    pub fn frame_swapped(&self) {
        let now = Instant::now();
        let mut s = self.st();
        s.frame_counter += 1;
        let sample_mem = s.frame_counter % 8 == 0;
        let last = s.last_swap.replace(now);
        let Some(cur) = s.current.clone() else { return };
        let mem = if sample_mem { Some(metrics::mem()) } else { None };
        if let Some(p) = s.phases.get_mut(&cur) {
            p.frames += 1;
            if let Some(l) = last {
                p.deltas_ms.push(now.duration_since(l).as_secs_f32() * 1000.0);
            }
            if let Some(m) = mem {
                p.peak_ws = p.peak_ws.max(m.working_set);
                p.peak_private = p.peak_private.max(m.private_ws);
            }
        }
    }

    pub fn log(&self, msg: &QString) {
        println!("T2 {msg}");
    }

    pub fn report(&self) {
        let mut s = self.st();
        if let Some(cur) = s.current.take() {
            if let Some(p) = s.phases.get_mut(&cur) {
                close(p);
            }
        }
        let m = metrics::mem();
        println!("T2 {}", crate::image::stats_string());
        println!(
            "T2 final ws_mb={:.1} pws_mb={:.1} peak_ws_mb={:.1}",
            metrics::mb(m.working_set),
            metrics::mb(m.private_ws),
            metrics::mb(m.peak_working_set)
        );
        for name in s.order.clone() {
            let p = &s.phases[&name];
            let mut d = p.deltas_ms.clone();
            d.sort_by(|a, b| a.total_cmp(b));
            let pct = |q: f64| -> f32 {
                if d.is_empty() {
                    0.0
                } else {
                    d[(((d.len() - 1) as f64) * q).round() as usize]
                }
            };
            let over_20 = d.iter().filter(|x| **x > 20.0).count();
            let over_50 = d.iter().filter(|x| **x > 50.0).count();
            println!(
                "T2 phase {name}: wall_ms={:.0} cpu_ms={:.0} cpu_pct={:.1} frames={} dt_p50={:.1} dt_p95={:.1} dt_p99={:.1} dt_max={:.1} n>20ms={} n>50ms={} ws_start_mb={:.1} ws_end_mb={:.1} peak_ws_mb={:.1} peak_pws_mb={:.1}",
                p.wall_ms,
                p.cpu_ms,
                100.0 * p.cpu_ms / p.wall_ms.max(1.0),
                p.frames,
                pct(0.5),
                pct(0.95),
                pct(0.99),
                d.last().copied().unwrap_or(0.0),
                over_20,
                over_50,
                metrics::mb(p.ws_start),
                metrics::mb(p.ws_end),
                metrics::mb(p.peak_ws),
                metrics::mb(p.peak_private),
            );
        }
    }
}
