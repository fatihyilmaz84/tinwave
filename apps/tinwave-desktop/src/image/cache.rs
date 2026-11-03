//! Bounded, byte-budgeted LRU cache of rendered thumbnails.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::Arc;

/// Least-recently-used cache bounded by the total size of its values in bytes.
///
/// Every operation is O(log n): recency is a monotonically increasing stamp kept in an ordered map, so the oldest
/// entry is the first key. Not thread-safe by itself; the provider core wraps it in a `Mutex`.
#[derive(Debug)]
pub struct ThumbnailCache<K> {
    max_bytes: usize,
    bytes: usize,
    next_stamp: u64,
    entries: HashMap<K, (Arc<[u8]>, u64)>,
    by_age: BTreeMap<u64, K>,
}

impl<K: Eq + Hash + Clone> ThumbnailCache<K> {
    /// An empty cache holding at most `max_bytes` bytes of pixel data.
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            bytes: 0,
            next_stamp: 0,
            entries: HashMap::new(),
            by_age: BTreeMap::new(),
        }
    }

    /// The cached value for `key`, marking it most recently used.
    pub fn get(&mut self, key: &K) -> Option<Arc<[u8]>> {
        let stamp = self.next_stamp;
        let (value, old_stamp) = self.entries.get_mut(key)?;
        self.next_stamp += 1;
        self.by_age.remove(old_stamp);
        *old_stamp = stamp;
        self.by_age.insert(stamp, key.clone());
        Some(Arc::clone(value))
    }

    /// Stores `value`, evicting the least recently used entries until it fits. A value larger than the whole budget
    /// is not stored (it would evict everything for one entry).
    pub fn insert(&mut self, key: K, value: Arc<[u8]>) {
        if value.len() > self.max_bytes {
            return;
        }
        if let Some((old, old_stamp)) = self.entries.remove(&key) {
            self.bytes -= old.len();
            self.by_age.remove(&old_stamp);
        }
        while self.bytes + value.len() > self.max_bytes {
            let Some((_, oldest)) = self.by_age.pop_first() else {
                break;
            };
            if let Some((evicted, _)) = self.entries.remove(&oldest) {
                self.bytes -= evicted.len();
            }
        }
        let stamp = self.next_stamp;
        self.next_stamp += 1;
        self.bytes += value.len();
        self.by_age.insert(stamp, key.clone());
        self.entries.insert(key, (value, stamp));
    }

    /// Bytes of pixel data currently held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Number of cached thumbnails.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(n: usize, fill: u8) -> Arc<[u8]> {
        vec![fill; n].into()
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let mut c = ThumbnailCache::new(30);
        c.insert("a", px(10, 1));
        c.insert("b", px(10, 2));
        c.insert("c", px(10, 3));
        assert!(c.get(&"a").is_some()); // a is now the most recent; b is the oldest
        c.insert("d", px(10, 4));
        assert!(c.get(&"b").is_none(), "b was the least recently used");
        assert!(c.get(&"a").is_some() && c.get(&"c").is_some() && c.get(&"d").is_some());
        assert_eq!(c.bytes(), 30);
    }

    #[test]
    fn never_exceeds_the_byte_budget() {
        let mut c = ThumbnailCache::new(100);
        for i in 0..1000u32 {
            c.insert(i, px(7 + (i as usize % 13), 0));
            assert!(c.bytes() <= 100, "after {i}: {}", c.bytes());
        }
        assert!(!c.is_empty());
    }

    #[test]
    fn replacing_a_key_does_not_double_count() {
        let mut c = ThumbnailCache::new(100);
        c.insert("a", px(40, 1));
        c.insert("a", px(30, 2));
        assert_eq!((c.len(), c.bytes()), (1, 30));
        assert_eq!(c.get(&"a").as_deref(), Some(&[2u8; 30][..]));
    }

    #[test]
    fn a_value_larger_than_the_budget_is_not_stored_and_evicts_nothing() {
        let mut c = ThumbnailCache::new(20);
        c.insert("a", px(10, 1));
        c.insert("big", px(21, 2));
        assert!(c.get(&"big").is_none());
        assert!(c.get(&"a").is_some());
    }
}
