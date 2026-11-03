//! The Rust half of one provider: the injected source, the thumbnail cache and the counters. Plain safe Rust; the
//! C++ shim reaches it only through the functions in `bridge`.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError};

use super::cache::ThumbnailCache;
use super::source::ArtworkSource;

/// Byte budget of the in-memory thumbnail cache of one provider.
///
/// Spec section 12 allows 80-160 MB while browsing, with Qt itself taking 65-75 MB of the idle baseline; the T-0002
/// spike measured a 8 MB cache holding about 72 percent of the requests of a 100 000-row traversal while the
/// browsing peak stayed at 123 MB working set. A 48 px thumbnail at 150 percent scaling is 9 KB, so 8 MB holds about
/// 900 of them. QML's own pixmap cache is bypassed (`cache: false` on the `Image`s of the future artwork component) so this is the only cache.
pub const THUMBNAIL_CACHE_BYTES: usize = 8 * 1024 * 1024;

/// Largest accepted thumbnail edge in pixels (a 1024 px square is 4 MiB, half the cache). Larger requests are
/// refused instead of allocating on a QML property's say-so.
pub const MAX_THUMBNAIL_EDGE: u32 = 1024;

/// Cache key: artwork key plus the exact pixel size asked for (size-specific thumbnails).
type Key = (Box<str>, u32, u32);

/// Counters of one provider, read by tests and the stress tool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoreStats {
    /// Thumbnail requests that reached Rust (cancelled tasks never get here).
    pub requests: u64,
    /// Requests answered from the cache.
    pub cache_hits: u64,
    /// Calls into the [`ArtworkSource`].
    pub source_calls: u64,
    /// Source results that were `None` or had the wrong length.
    pub misses: u64,
    /// Bytes currently cached.
    pub cached_bytes: usize,
    /// Thumbnails currently cached.
    pub cached_entries: usize,
}

/// Source + bounded cache + counters for one provider. Shared (`Arc`) between the UI owner's handle and the C++
/// state; all methods take `&self` and are called from worker threads.
pub struct ProviderCore {
    source: Arc<dyn ArtworkSource>,
    cache: Mutex<ThumbnailCache<Key>>,
    requests: AtomicU64,
    cache_hits: AtomicU64,
    source_calls: AtomicU64,
    misses: AtomicU64,
}

impl fmt::Debug for ProviderCore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderCore")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl ProviderCore {
    /// A core with the production cache budget.
    pub fn new(source: Arc<dyn ArtworkSource>) -> Self {
        Self::with_cache_bytes(source, THUMBNAIL_CACHE_BYTES)
    }

    /// A core with an explicit cache budget (tests).
    pub fn with_cache_bytes(source: Arc<dyn ArtworkSource>, cache_bytes: usize) -> Self {
        Self {
            source,
            cache: Mutex::new(ThumbnailCache::new(cache_bytes)),
            requests: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            source_calls: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// The `width` x `height` thumbnail for `key`: from the cache, else from the source (then cached). `None` when
    /// the size is out of range or the source has no (valid) artwork. The source runs outside the cache lock, so a
    /// slow decode never blocks other workers' cache hits.
    pub fn thumbnail(&self, key: &str, width: u32, height: u32) -> Option<Arc<[u8]>> {
        if width == 0 || height == 0 || width > MAX_THUMBNAIL_EDGE || height > MAX_THUMBNAIL_EDGE {
            return None;
        }
        self.requests.fetch_add(1, Relaxed);
        let cache_key: Key = (key.into(), width, height);
        if let Some(hit) = self.lock_cache().get(&cache_key) {
            self.cache_hits.fetch_add(1, Relaxed);
            return Some(hit);
        }
        self.source_calls.fetch_add(1, Relaxed);
        // At most 1024 * 1024 * 4, so the product cannot overflow usize.
        let expected = width as usize * height as usize * 4;
        let pixels: Option<Arc<[u8]>> = self
            .source
            .thumbnail(key, width, height)
            .filter(|p| p.len() == expected)
            .map(Arc::from);
        match pixels {
            Some(p) => {
                self.lock_cache().insert(cache_key, Arc::clone(&p));
                Some(p)
            }
            None => {
                self.misses.fetch_add(1, Relaxed);
                None
            }
        }
    }

    /// A snapshot of the counters.
    pub fn stats(&self) -> CoreStats {
        let cache = self.lock_cache();
        CoreStats {
            requests: self.requests.load(Relaxed),
            cache_hits: self.cache_hits.load(Relaxed),
            source_calls: self.source_calls.load(Relaxed),
            misses: self.misses.load(Relaxed),
            cached_bytes: cache.bytes(),
            cached_entries: cache.len(),
        }
    }

    // The cache stays consistent if a holder panicked (no operation panics half way), so poisoning is ignored.
    fn lock_cache(&self) -> std::sync::MutexGuard<'_, ThumbnailCache<Key>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Returns a solid-colour thumbnail whose first byte encodes the key length; `None` for the key "none".
    #[derive(Default)]
    struct CountingSource {
        calls: AtomicUsize,
        wrong_length: bool,
    }

    impl ArtworkSource for CountingSource {
        fn thumbnail(&self, key: &str, width: u32, height: u32) -> Option<Vec<u8>> {
            self.calls.fetch_add(1, Relaxed);
            if key == "none" {
                return None;
            }
            let n = width as usize * height as usize * 4 - usize::from(self.wrong_length);
            Some(vec![key.len() as u8; n])
        }
    }

    fn core(source: &Arc<CountingSource>, cache_bytes: usize) -> ProviderCore {
        ProviderCore::with_cache_bytes(Arc::<CountingSource>::clone(source), cache_bytes)
    }

    #[test]
    fn repeated_requests_hit_the_cache_and_call_the_source_once() {
        let src = Arc::new(CountingSource::default());
        let c = core(&src, 1 << 20);
        for _ in 0..5 {
            assert_eq!(c.thumbnail("abc", 8, 8).map(|p| p.len()), Some(8 * 8 * 4));
        }
        assert_eq!(src.calls.load(Relaxed), 1);
        let s = c.stats();
        assert_eq!((s.requests, s.cache_hits, s.source_calls), (5, 4, 1));
    }

    #[test]
    fn thumbnails_are_size_specific() {
        let src = Arc::new(CountingSource::default());
        let c = core(&src, 1 << 20);
        assert_eq!(c.thumbnail("a", 8, 8).map(|p| p.len()), Some(256));
        assert_eq!(c.thumbnail("a", 12, 12).map(|p| p.len()), Some(576));
        assert_eq!(src.calls.load(Relaxed), 2);
    }

    #[test]
    fn no_artwork_and_malformed_results_are_misses_and_are_not_cached() {
        let src = Arc::new(CountingSource::default());
        let c = core(&src, 1 << 20);
        assert!(c.thumbnail("none", 8, 8).is_none());
        assert!(c.thumbnail("none", 8, 8).is_none());
        assert_eq!(
            src.calls.load(Relaxed),
            2,
            "a miss is asked again (artwork may appear later)"
        );

        let bad = Arc::new(CountingSource {
            wrong_length: true,
            ..CountingSource::default()
        });
        let c = core(&bad, 1 << 20);
        assert!(c.thumbnail("a", 8, 8).is_none());
        assert_eq!(c.stats().misses, 1);
        assert_eq!(c.stats().cached_entries, 0);
    }

    #[test]
    fn out_of_range_sizes_are_refused_without_calling_the_source() {
        let src = Arc::new(CountingSource::default());
        let c = core(&src, 1 << 20);
        assert!(c.thumbnail("a", 0, 8).is_none());
        assert!(c.thumbnail("a", 8, 0).is_none());
        assert!(c.thumbnail("a", MAX_THUMBNAIL_EDGE + 1, 8).is_none());
        assert_eq!(src.calls.load(Relaxed), 0);
        assert_eq!(c.stats().requests, 0);
    }

    #[test]
    fn the_cache_stays_within_its_budget() {
        let src = Arc::new(CountingSource::default());
        let budget = 10 * 256;
        let c = core(&src, budget);
        for i in 0..100 {
            let key = format!("k{i}");
            assert!(c.thumbnail(&key, 8, 8).is_some());
            assert!(c.stats().cached_bytes <= budget);
        }
        assert_eq!(c.stats().cached_entries, 10);
    }
}
