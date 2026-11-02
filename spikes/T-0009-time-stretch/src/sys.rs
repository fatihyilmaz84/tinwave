//! Tiny Windows helpers without pulling in the `windows` crate.
#![allow(non_snake_case)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThread() -> isize;
    fn GetCurrentProcess() -> isize;
    fn SetThreadAffinityMask(h: isize, mask: usize) -> usize;
    fn SetThreadPriority(h: isize, p: i32) -> i32;
    fn SetPriorityClass(h: isize, c: u32) -> i32;
    fn K32GetProcessMemoryInfo(h: isize, c: *mut PMC, cb: u32) -> i32;
    fn GetThreadTimes(h: isize, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> i32;
}
extern "C" {
    fn tw_cpp_allocs_get() -> u64;
}
/// Total C++ `operator new` calls so far (replacement hook in newhook.cpp).
pub fn cpp_allocs() -> u64 {
    unsafe { tw_cpp_allocs_get() }
}
#[link(name = "winmm")]
extern "system" {
    pub fn timeBeginPeriod(ms: u32) -> u32;
}

#[repr(C)]
struct PMC {
    cb: u32,
    page_faults: u32,
    peak_ws: usize,
    ws: usize,
    qp_peak_paged: usize,
    qp_paged: usize,
    qp_peak_np: usize,
    qp_np: usize,
    pagefile: usize,
    peak_pagefile: usize,
}

pub fn pin_to_core(core: usize) {
    unsafe {
        SetThreadAffinityMask(GetCurrentThread(), 1usize << core);
    }
}
pub fn high_priority() {
    unsafe {
        SetPriorityClass(GetCurrentProcess(), 0x80); // HIGH_PRIORITY_CLASS
        SetThreadPriority(GetCurrentThread(), 2); // THREAD_PRIORITY_HIGHEST
    }
}
/// (working set, private bytes (pagefile usage)) in bytes.
pub fn mem() -> (usize, usize) {
    unsafe {
        let mut p: PMC = std::mem::zeroed();
        p.cb = std::mem::size_of::<PMC>() as u32;
        K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb);
        (p.ws, p.pagefile)
    }
}
/// Thread CPU time (kernel+user) in seconds (100 ns units, coarse tick granularity on some systems).
pub fn thread_cpu_secs() -> f64 {
    unsafe {
        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
        GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u);
        (k + u) as f64 * 1e-7
    }
}

// ---- counting allocator: lets tests assert zero allocations inside the RT callback
pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
pub static RT_ALLOCS: AtomicU64 = AtomicU64::new(0);
thread_local! { pub static IN_RT: Cell<bool> = const { Cell::new(false) }; }

pub struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        if IN_RT.with(|c| c.get()) {
            RT_ALLOCS.fetch_add(1, Relaxed);
        }
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if IN_RT.with(|c| c.get()) {
            RT_ALLOCS.fetch_add(1, Relaxed);
        }
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        if IN_RT.with(|c| c.get()) {
            RT_ALLOCS.fetch_add(1, Relaxed);
        }
        System.realloc(p, l, n)
    }
}
