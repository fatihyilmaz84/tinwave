//! CMake-driven variant of the T-0002 spike: same QObjects, built as a staticlib linked into a C++ main.
mod bench;
mod counters;
mod image;
mod metrics;
mod model;

/// Called first thing from the C++ `main`.
#[no_mangle]
pub extern "C" fn tw_init_metrics() {
    metrics::init();
}
