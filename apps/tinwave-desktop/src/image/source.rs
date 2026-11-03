//! Where artwork pixels come from. The provider owns no library knowledge: it asks an injected [`ArtworkSource`].
//! The library-backed source arrives with the library tickets; tests and the stress tool inject their own.

use std::fmt;

/// Produces a thumbnail for an artwork key at an exact pixel size.
///
/// Called on the provider's worker threads (never on the Qt GUI thread, never on the playback path), so it may
/// block on disk IO or decoding. It must not call back into Qt.
pub trait ArtworkSource: Send + Sync + 'static {
    /// The thumbnail for `key` (the part of the `image://art/<key>` URL after the provider name, opaque to the
    /// provider) as `width * height` pixels in `QImage::Format_RGB32`: 4 bytes per pixel, rows top to bottom, bytes
    /// B, G, R, 0xFF (opaque).
    ///
    /// Returns `None` when there is no artwork for `key`; QML then sees an `Image` error and shows its placeholder.
    /// A result whose length is not `width * height * 4` is treated as `None` and counted as a source error.
    fn thumbnail(&self, key: &str, width: u32, height: u32) -> Option<Vec<u8>>;
}

/// The source `main` wires until the library exists: it has no artwork for any key.
#[derive(Clone, Copy, Default)]
pub struct NoArtwork;

impl fmt::Debug for NoArtwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoArtwork")
    }
}

impl ArtworkSource for NoArtwork {
    fn thumbnail(&self, _key: &str, _width: u32, _height: u32) -> Option<Vec<u8>> {
        None
    }
}
