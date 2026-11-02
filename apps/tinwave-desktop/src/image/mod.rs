//! Artwork: thumbnails for QML `Image { source: "image://art/<key>" }`.
//!
//! * [`source`]: the injected [`ArtworkSource`] (the library-backed one arrives with the library tickets).
//! * [`cache`], [`provider_core`]: the bounded in-memory thumbnail cache and the per-provider state, plain safe Rust.
//!
//! The C++ shim binding (`cpp/image_provider.cpp`) and the safe `ArtworkProvider` handle are in
//! `crate::app::artwork_bridge` (a CXX-Qt build constraint, see there).
//!
//! The provider follows the six-rule lifetime pattern of ADR 0008 ("image provider lifetime") and
//! `docs/rules/qml-ui.md`; the rules are written out next to the code that implements them, in the shim.

pub mod cache;
pub mod provider_core;
pub mod source;

pub use provider_core::{CoreStats, ProviderCore, MAX_THUMBNAIL_EDGE, THUMBNAIL_CACHE_BYTES};
pub use source::{ArtworkSource, NoArtwork};

/// Name of the provider in QML URLs: `image://art/<key>`.
pub const PROVIDER_NAME: &str = "art";

/// Worker threads of the production provider: at most this many thumbnails decode at once, so scrolling never
/// starves the rest of the app of cores (spec section 12: low CPU while browsing).
pub const POOL_THREADS: i32 = 2;
