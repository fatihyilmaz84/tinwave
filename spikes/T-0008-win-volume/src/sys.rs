//! Small OS helpers: QPC timestamps (system-wide, comparable across processes), CPU/RSS sampling.

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

pub fn qpc() -> i64 {
    let mut v = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut v);
    }
    v
}

pub fn qpc_freq() -> i64 {
    let mut v = 0i64;
    unsafe {
        let _ = QueryPerformanceFrequency(&mut v);
    }
    v.max(1)
}

pub fn qpc_to_ms(delta: i64) -> f64 {
    delta as f64 * 1000.0 / qpc_freq() as f64
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

#[derive(Debug, Clone, Copy)]
pub struct ProcStats {
    /// kernel+user CPU time in 100 ns units
    pub cpu_100ns: u64,
    pub working_set: usize,
    pub private_bytes: usize,
    pub peak_working_set: usize,
}

pub fn proc_stats() -> ProcStats {
    unsafe {
        let h = GetCurrentProcess();
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        let mut m = PROCESS_MEMORY_COUNTERS_EX::default();
        m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let _ = K32GetProcessMemoryInfo(h, &mut m as *mut _ as *mut _, m.cb);
        ProcStats {
            cpu_100ns: ft(k) + ft(u),
            working_set: m.WorkingSetSize,
            private_bytes: m.PrivateUsage,
            peak_working_set: m.PeakWorkingSetSize,
        }
    }
}
