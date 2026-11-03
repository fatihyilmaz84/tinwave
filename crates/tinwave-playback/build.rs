//! Compiles the C ABI over the vendored Signalsmith Stretch headers (T-0026, ADR 0006).
//!
//! Only a C++ compiler is needed (MSVC on Windows, the system compiler elsewhere): the headers are
//! checked in under `vendor/signalsmith/`, so there is no bindgen and no libclang.

use std::path::Path;

const VENDOR: &str = "vendor/signalsmith";
const WRAPPER: &str = "src/stretch/tw_stretch.cpp";
const WRAPPER_HEADER: &str = "src/stretch/tw_stretch.h";
const ALLOC_PROBE: &str = "src/stretch/alloc_probe.cpp";

fn base() -> cc::Build {
    let mut build = cc::Build::new();
    build.cpp(true).std("c++14").warnings(true);
    // The wrapper catches C++ exceptions at the FFI boundary, which needs exception unwinding.
    if build.get_compiler().is_like_msvc() {
        build.flag("/EHsc");
    }
    build
}

fn main() {
    for path in [VENDOR, WRAPPER, WRAPPER_HEADER, ALLOC_PROBE] {
        println!("cargo:rerun-if-changed={path}");
    }

    let mut stretch = base();
    stretch
        .file(WRAPPER)
        // The stretcher runs on the decoder thread at about 1 % of a core when optimised, but an
        // unoptimised C++ build would make every test and dev run several times slower for no
        // benefit: the C++ side is third-party code that nobody steps through. Optimise it
        // whatever the profile.
        .opt_level(2);
    // The vendored headers are pinned third-party code: their warnings are not ours to fix, and
    // they would bury the ones in our wrapper. Include them as system headers where the compiler
    // can (an old MSVC without /external falls back to a plain include and prints them).
    if !stretch.get_compiler().is_like_msvc() {
        stretch.flag(format!("-isystem{VENDOR}"));
    } else if stretch.is_flag_supported("/external:W0").unwrap_or(false) {
        stretch
            .flag("/external:W0")
            .flag(format!("/external:I{VENDOR}"));
    } else {
        stretch.include(Path::new(VENDOR));
    }
    stretch.compile("tinwave_stretch");

    // The allocation probe replaces the global `operator new`, so it must never reach the
    // application: it is handed to the linker for test binaries only.
    for object in base()
        .file(ALLOC_PROBE)
        .opt_level(2)
        .compile_intermediates()
    {
        println!("cargo:rustc-link-arg-tests={}", object.display());
    }
}
