//! Spike-only process metrics via windows-sys. Not production code.
use std::sync::OnceLock;
use std::time::Instant;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

static START: OnceLock<Instant> = OnceLock::new();

pub fn init() {
    let _ = START.set(Instant::now());
}

pub fn since_main_ms() -> f64 {
    START.get().map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0)
}

fn ft(f: &FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

/// (creation, exit, kernel, user) FILETIMEs of this process.
fn times() -> (u64, u64, u64) {
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (z, z, z, z);
    // SAFETY: valid out pointers, current-process pseudo handle.
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    (ft(&c), ft(&k), ft(&u))
}

/// Milliseconds since the OS created this process.
pub fn process_age_ms() -> f64 {
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let mut now = z;
    // SAFETY: valid out pointer.
    unsafe { GetSystemTimeAsFileTime(&mut now) };
    (ft(&now).saturating_sub(times().0)) as f64 / 10_000.0
}

/// Total CPU time (kernel + user) of this process in ms.
pub fn cpu_ms() -> f64 {
    let (_, k, u) = times();
    (k + u) as f64 / 10_000.0
}

#[derive(Clone, Copy, Default)]
pub struct Mem {
    pub working_set: u64,
    pub private: u64,
    pub peak_working_set: u64,
    /// Task Manager "Memory" column (private working set).
    pub private_ws: u64,
}

pub fn mem() -> Mem {
    // SAFETY: zeroed POD struct, size set, current-process pseudo handle.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS_EX2 = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
        K32GetProcessMemoryInfo(GetCurrentProcess(), (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX2).cast(), c.cb);
        Mem {
            working_set: c.WorkingSetSize as u64,
            private: c.PrivateUsage as u64,
            peak_working_set: c.PeakWorkingSetSize as u64,
            private_ws: c.PrivateWorkingSetSize as u64,
        }
    }
}

pub fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

/// Spike-only crash diagnostics: print the faulting module+RVA and a raw stack on an access violation.
pub fn install_crash_handler() {
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, RtlCaptureStackBackTrace, EXCEPTION_POINTERS,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    fn describe(addr: usize) -> String {
        // SAFETY: out pointers are valid; FROM_ADDRESS treats the argument as an address.
        unsafe {
            let mut h = std::ptr::null_mut();
            let ok = GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                addr as *const u16,
                &mut h,
            );
            if ok == 0 {
                return format!("{addr:#x}");
            }
            let mut buf = [0u16; 260];
            let n = GetModuleFileNameW(h, buf.as_mut_ptr(), 260) as usize;
            let name = String::from_utf16_lossy(&buf[..n]);
            let short = name.rsplit('\\').next().unwrap_or("?");
            format!("{short}+{:#x}", addr - h as usize)
        }
    }

    unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the OS passes a valid EXCEPTION_POINTERS.
        unsafe {
            let rec = &*(*info).ExceptionRecord;
            if rec.ExceptionCode as u32 != 0xC000_0005 {
                return 0; // EXCEPTION_CONTINUE_SEARCH
            }
            let tid = std::thread::current();
            eprintln!(
                "T2 CRASH access violation at {} on thread {:?} ({}), target={:#x}",
                describe(rec.ExceptionAddress as usize),
                tid.id(),
                tid.name().unwrap_or("?"),
                rec.ExceptionInformation[1]
            );
            let mut frames = [std::ptr::null_mut(); 32];
            let n = RtlCaptureStackBackTrace(0, 32, frames.as_mut_ptr(), std::ptr::null_mut());
            for f in &frames[..n as usize] {
                eprintln!("T2 CRASH   frame {}", describe(*f as usize));
            }
        }
        0
    }
    // SAFETY: registering a process-wide first-chance handler that only reads and prints.
    unsafe { AddVectoredExceptionHandler(1, Some(handler)) };
}
