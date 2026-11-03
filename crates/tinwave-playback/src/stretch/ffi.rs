//! Raw declarations of the C ABI in `tw_stretch.h`. Keep the two in step by hand; `tests/stretch.rs`
//! exercises every function, so a mismatch shows up as a failing test, not as silent corruption.

use std::marker::{PhantomData, PhantomPinned};

/// The C++ `tw_stretch` object. Only ever used behind a pointer; never constructed in Rust.
#[repr(C)]
#[derive(Debug)]
pub(super) struct TwStretch {
    _opaque: [u8; 0],
    // Not `Send`/`Sync`/`Unpin` on its own account: only the wrapper decides what is safe.
    _marker: PhantomData<(*mut u8, PhantomPinned)>,
}

extern "C" {
    pub(super) fn tw_stretch_create(channels: i32, sample_rate: f32) -> *mut TwStretch;
    pub(super) fn tw_stretch_destroy(handle: *mut TwStretch);
    pub(super) fn tw_stretch_reset(handle: *mut TwStretch);
    pub(super) fn tw_stretch_input_latency(handle: *const TwStretch) -> usize;
    pub(super) fn tw_stretch_output_latency(handle: *const TwStretch) -> usize;
    pub(super) fn tw_stretch_process(
        handle: *mut TwStretch,
        input: *const f32,
        input_frames: usize,
        output: *mut f32,
        output_frames: usize,
    ) -> i32;
    pub(super) fn tw_stretch_flush(
        handle: *mut TwStretch,
        output: *mut f32,
        output_frames: usize,
    ) -> i32;
}
