//! The only `unsafe` of the artwork provider: the cxx declarations of the C++ shim (`cpp/image_provider.cpp`), the two
//! `extern "C"` functions it calls back into, and the safe [`ArtworkProvider`] wrapper around them.
//!
//! CXX-Qt 0.10 binds `QQmlEngine::addImageProvider` but cannot subclass `QQuickAsyncImageProvider` /
//! `QQuickImageResponse`, hence the shim (ADR 0008 decision 4). Its lifetime rules are in `image_provider.cpp`.
//!
//! This file lives in `app/` and not in `image/` because cxx-qt-build requires every bridge file of a QML module to
//! sit in one directory (Qt bug QTBUG-93443) and `app_info.rs` is already here. The provider's logic (source, cache,
//! counters) is in `crate::image`.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlimageproviderbase.h");
        type QQmlImageProviderBase = cxx_qt_lib::QQmlImageProviderBase;

        include!("image_provider.h");
        /// Owning handle to the provider's shared C++ state; outlives the QML engine's copy of the provider.
        type ProviderHandle;

        /// Creates the shared state. `rust_ctx` is an `Arc<ProviderCore>` strong count (`Arc::into_raw`, typed as bytes
        /// because cxx has no `c_void`) that the new state owns and gives back through `tw_provider_ctx_release`.
        ///
        /// # Safety
        /// `rust_ctx` must come from `Arc::into_raw` of an `Arc<ProviderCore>` and not be used by the caller again.
        unsafe fn tw_provider_handle_new(
            rust_ctx: *const u8,
            pool_threads: i32,
        ) -> UniquePtr<ProviderHandle>;
        /// Creates the C++ provider; ownership passes to whoever receives the pointer (the QML engine).
        fn tw_provider_make(handle: &ProviderHandle) -> *mut QQmlImageProviderBase;
        /// GUI thread, after the event loop ended and before the QML engine is dropped: set `closing` and wait for
        /// every worker thread (ADR 0008 addendum, rule 5). Idempotent.
        fn tw_provider_quiesce(handle: &ProviderHandle);
        /// One of the shim's counters (0 responses, 1 finished, 2 duplicates, 3 cancels, 4 tasks run, 5 skipped).
        fn tw_provider_counter(handle: &ProviderHandle, which: u32) -> u64;
    }
}

use std::ffi::c_void;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;

use cxx::UniquePtr;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QQmlApplicationEngine, QQmlEngine, QString};

use crate::image::{ArtworkSource, CoreStats, ProviderCore};

/// Counters of the C++ shim (see `cpp/thumb_task.h`), plus those of the Rust core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderStats {
    /// Image responses created by Qt's reader thread.
    pub responses: u64,
    /// `finished()` emitted. Equals `responses` once every request settled; lower while some are still in flight
    /// when the provider is shut down.
    pub finished: u64,
    /// Second `finished()` attempts that were suppressed. Always 0: a second emission crashes Qt (T-0041 crash A).
    pub duplicate_finishes: u64,
    /// `cancel()` calls.
    pub cancels: u64,
    /// Tasks that asked Rust for pixels.
    pub tasks_run: u64,
    /// Tasks that skipped the work because they were cancelled or the provider was closing.
    pub tasks_skipped: u64,
    /// The Rust core's counters.
    pub core: CoreStats,
}

/// Handle to one installed artwork provider. The provider object itself belongs to the QML engine; this handle keeps
/// the shared state (pool, closing flag, counters, Rust core) alive and is how the UI owner stops the workers.
pub struct ArtworkProvider {
    handle: UniquePtr<ffi::ProviderHandle>,
    core: Arc<ProviderCore>,
}

impl fmt::Debug for ArtworkProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArtworkProvider")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl ArtworkProvider {
    /// Registers an async image provider named `name` (`image://<name>/<key>`) on `engine`, backed by `source`, with
    /// a pool of `pool_threads` workers (at least 1; the thread count is the bound on concurrent decodes).
    ///
    /// The engine takes ownership of the provider object. The caller must keep the returned handle and call
    /// [`quiesce`](Self::quiesce) after the event loop ended and before the engine is dropped (the UI owner does).
    pub fn install(
        engine: Pin<&mut QQmlApplicationEngine>,
        name: &str,
        source: Arc<dyn ArtworkSource>,
        pool_threads: i32,
    ) -> Self {
        let core = Arc::new(ProviderCore::new(source));
        let ctx = Arc::into_raw(Arc::clone(&core)).cast::<u8>();
        // SAFETY: `ctx` is the strong count just leaked by `Arc::into_raw`; the C++ state now owns it and returns it
        // exactly once through `tw_provider_ctx_release`, when its last user is gone.
        let handle = unsafe { ffi::tw_provider_handle_new(ctx, pool_threads) };
        let provider = ffi::tw_provider_make(&handle);
        let qml_engine: Pin<&mut QQmlEngine> = engine.upcast_pin();
        // SAFETY: `provider` is a freshly allocated provider that nothing else owns; the engine takes ownership and
        // deletes it. The shared state it uses is kept alive by the `handle` stored below, so the owner can still
        // quiesce after the engine is gone.
        unsafe { qml_engine.add_image_provider(&QString::from(name), provider) };
        Self { handle, core }
    }

    /// Stops the workers: sets the closing flag and waits until none is running. GUI thread only; idempotent. After
    /// it, queued tasks complete empty without calling the source and late requests start no work.
    pub fn quiesce(&self) {
        ffi::tw_provider_quiesce(&self.handle);
    }

    /// A snapshot of the shim's and the core's counters.
    pub fn stats(&self) -> ProviderStats {
        let counter = |which| ffi::tw_provider_counter(&self.handle, which);
        ProviderStats {
            responses: counter(0),
            finished: counter(1),
            duplicate_finishes: counter(2),
            cancels: counter(3),
            tasks_run: counter(4),
            tasks_skipped: counter(5),
            core: self.core.stats(),
        }
    }
}

/// Called by the C++ task on a pool thread: renders (or fetches from the cache) the `w` x `h` thumbnail of the UTF-8
/// key `id[..id_len]` into `out` (`w * h * 4` bytes, `QImage::Format_RGB32`). Returns 1 on success, 0 when there is
/// no artwork. Never unwinds into C++: a panic in the source is reported by the panic hook and counts as no artwork.
///
/// # Safety
/// `ctx` must be the pointer given to `tw_provider_handle_new` and still owned by the C++ state (not yet released),
/// `id` must point to `id_len` readable bytes, and `out` to `w * h * 4` writable bytes when `w` and `h` are positive.
#[no_mangle]
pub unsafe extern "C" fn tw_render_thumbnail(
    ctx: *const c_void,
    id: *const u8,
    id_len: usize,
    w: i32,
    h: i32,
    out: *mut u8,
) -> i32 {
    let (Ok(width), Ok(height)) = (u32::try_from(w), u32::try_from(h)) else {
        return 0;
    };
    // SAFETY: `ctx` is a live `Arc<ProviderCore>` (caller contract) and only shared references are made from it.
    let core = unsafe { &*ctx.cast::<ProviderCore>() };
    // SAFETY: `id` points to `id_len` readable bytes (caller contract).
    let id = unsafe { std::slice::from_raw_parts(id, id_len) };
    let Ok(key) = std::str::from_utf8(id) else {
        return 0;
    };
    let Ok(Some(pixels)) = catch_unwind(AssertUnwindSafe(|| core.thumbnail(key, width, height)))
    else {
        return 0;
    };
    // `thumbnail` only returns buffers of exactly width * height * 4 bytes, and width/height are in range.
    // SAFETY: `out` has room for `w * h * 4` bytes (caller contract) and `pixels` has exactly that length; the two
    // buffers are distinct allocations.
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), out, pixels.len()) };
    1
}

/// Gives back the strong count handed over in `tw_provider_handle_new`.
///
/// # Safety
/// `ctx` must be that pointer and must be released exactly once; no worker may use it afterwards.
#[no_mangle]
pub unsafe extern "C" fn tw_provider_ctx_release(ctx: *const c_void) {
    // SAFETY: caller contract: `ctx` came from `Arc::into_raw::<ProviderCore>` and is released once.
    drop(unsafe { Arc::from_raw(ctx.cast::<ProviderCore>()) });
}
