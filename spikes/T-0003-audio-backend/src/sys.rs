//! Process metrics: CPU time and memory (Windows).
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Total CPU time (kernel + user) of this process in seconds.
pub fn cpu_seconds() -> f64 {
    unsafe {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
        (ft(k) + ft(u)) as f64 / 1e7
    }
}

pub struct Mem {
    pub working_set_mb: f64,
    pub peak_working_set_mb: f64,
    pub private_mb: f64,
}

pub fn mem() -> Mem {
    unsafe {
        let mut m = PROCESS_MEMORY_COUNTERS_EX::default();
        m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut m as *mut _ as *mut _, m.cb);
        Mem {
            working_set_mb: m.WorkingSetSize as f64 / 1048576.0,
            peak_working_set_mb: m.PeakWorkingSetSize as f64 / 1048576.0,
            private_mb: m.PrivateUsage as f64 / 1048576.0,
        }
    }
}
