//! Counting global allocator for real-time-safety tests (dev-dependency only).
//!
//! Install [`CountingAlloc`] as the test binary's `#[global_allocator]`, then wrap the code that
//! must not touch the heap in [`count_allocations`]. Every allocation, zeroed allocation,
//! reallocation and free made by the calling thread inside the closure is counted; other threads
//! (a decoder thread, the test harness) are not, so the test thread can play the audio callback
//! while the rest of the system allocates freely.
//!
//! ```ignore
//! #[global_allocator]
//! static GLOBAL: tinwave_test_alloc::CountingAlloc = tinwave_test_alloc::CountingAlloc;
//!
//! let (frames, allocs) = tinwave_test_alloc::count_allocations(|| renderer.render(&mut out, 0));
//! assert_eq!(allocs, 0);
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static IN_REGION: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<u64> = const { Cell::new(0) };
}

/// Forwards to the system allocator and counts calls made inside [`count_allocations`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CountingAlloc;

fn note() {
    // `try_with`: during thread teardown the thread-locals may be gone; that is not the region.
    if IN_REGION.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNT.try_with(|n| n.set(n.get() + 1));
    }
}

// SAFETY: every method forwards its arguments unchanged to `System`, which upholds the
// `GlobalAlloc` contract; the counting touches only `const`-initialised `Cell` thread-locals and
// never allocates.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        System.alloc_zeroed(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        System.realloc(ptr, layout, new_size)
    }
}

struct RegionGuard;

impl Drop for RegionGuard {
    fn drop(&mut self) {
        IN_REGION.with(|r| r.set(false));
    }
}

/// Run `f` and return its result with the number of heap allocations plus frees the calling
/// thread made while it ran. Not reentrant. Only meaningful when [`CountingAlloc`] is the global
/// allocator.
pub fn count_allocations<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let before = COUNT.with(Cell::get);
    IN_REGION.with(|r| r.set(true));
    let guard = RegionGuard;
    let result = f();
    drop(guard);
    (result, COUNT.with(Cell::get) - before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[global_allocator]
    static GLOBAL: CountingAlloc = CountingAlloc;

    #[test]
    fn counts_alloc_and_free_inside_the_region_only() {
        let (_, outside) = count_allocations(|| ());
        assert_eq!(outside, 0);
        let before = COUNT.with(Cell::get);
        drop(Vec::<u8>::with_capacity(64));
        assert_eq!(
            COUNT.with(Cell::get),
            before,
            "outside a region nothing counts"
        );

        let (_, inside) = count_allocations(|| drop(Vec::<u8>::with_capacity(64)));
        assert_eq!(inside, 2, "one allocation and one free");
    }

    #[test]
    fn other_threads_are_not_counted() {
        let (_, n) = count_allocations(|| {
            let worker = std::thread::spawn(|| {
                for _ in 0..1_000 {
                    drop(std::hint::black_box(Vec::<u8>::with_capacity(64)));
                }
            });
            worker.join().ok()
        });
        // Spawning and joining allocate a handful of times on this thread; the 2000 allocations
        // and frees of the worker are not part of the count.
        assert!(n < 100, "{n}");
    }
}
