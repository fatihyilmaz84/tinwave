//! A small bounded least-recently-used map, used for dictionary lookups.

use std::collections::HashMap;
use std::hash::Hash;

const NIL: usize = usize::MAX;

struct Node<K, V> {
    key: K,
    value: V,
    prev: usize,
    next: usize,
}

/// Bounded LRU map with O(1) `get` and `insert`. Evicts the least recently used entry when a
/// new key arrives at capacity. Entries live in a slab, linked by index.
pub(crate) struct Lru<K, V> {
    capacity: usize,
    map: HashMap<K, usize>,
    nodes: Vec<Node<K, V>>,
    /// Most recently used.
    head: usize,
    /// Least recently used.
    tail: usize,
}

impl<K: Copy + Eq + Hash, V: Copy> Lru<K, V> {
    /// A cache holding at most `capacity` entries (at least 1).
    pub(crate) fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            map: HashMap::new(),
            nodes: Vec::new(),
            head: NIL,
            tail: NIL,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// The value for `key`, marking it most recently used.
    pub(crate) fn get(&mut self, key: &K) -> Option<V> {
        let index = *self.map.get(key)?;
        self.touch(index);
        Some(self.nodes[index].value)
    }

    /// Stores `value` for `key` as most recently used.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        if let Some(&index) = self.map.get(&key) {
            self.nodes[index].value = value;
            self.touch(index);
            return;
        }
        let index = if self.nodes.len() < self.capacity {
            self.nodes.push(Node {
                key,
                value,
                prev: NIL,
                next: NIL,
            });
            self.nodes.len() - 1
        } else {
            // Reuse the least recently used slot.
            let index = self.tail;
            self.unlink(index);
            self.map.remove(&self.nodes[index].key);
            self.nodes[index].key = key;
            self.nodes[index].value = value;
            index
        };
        self.map.insert(key, index);
        self.push_front(index);
    }

    fn touch(&mut self, index: usize) {
        if self.head != index {
            self.unlink(index);
            self.push_front(index);
        }
    }

    fn unlink(&mut self, index: usize) {
        let (prev, next) = (self.nodes[index].prev, self.nodes[index].next);
        if prev == NIL {
            if self.head == index {
                self.head = next;
            }
        } else {
            self.nodes[prev].next = next;
        }
        if next == NIL {
            if self.tail == index {
                self.tail = prev;
            }
        } else {
            self.nodes[next].prev = prev;
        }
        self.nodes[index].prev = NIL;
        self.nodes[index].next = NIL;
    }

    fn push_front(&mut self, index: usize) {
        self.nodes[index].prev = NIL;
        self.nodes[index].next = self.head;
        if self.head != NIL {
            self.nodes[self.head].prev = index;
        }
        self.head = index;
        if self.tail == NIL {
            self.tail = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used_entry() {
        let mut lru = Lru::new(3);
        lru.insert(1, 10);
        lru.insert(2, 20);
        lru.insert(3, 30);
        assert_eq!(lru.get(&1), Some(10), "touch 1: 2 is now the oldest");
        lru.insert(4, 40);
        assert_eq!(lru.get(&2), None);
        assert_eq!(
            (lru.get(&1), lru.get(&3), lru.get(&4)),
            (Some(10), Some(30), Some(40))
        );
        assert_eq!(lru.len(), 3);
    }

    #[test]
    fn stays_bounded_and_overwrites_in_place() {
        let mut lru = Lru::new(2);
        for i in 0..1_000u32 {
            lru.insert(i, i);
            assert!(lru.len() <= 2);
        }
        lru.insert(999, 7);
        assert_eq!(lru.get(&999), Some(7));
        assert_eq!(lru.get(&998), Some(998));
        assert_eq!(lru.len(), 2);
    }

    #[test]
    fn capacity_zero_holds_one_entry() {
        let mut lru = Lru::new(0);
        lru.insert(1, 1);
        lru.insert(2, 2);
        assert_eq!((lru.get(&1), lru.get(&2)), (None, Some(2)));
    }

    #[test]
    fn matches_a_naive_model_under_a_mixed_workload() {
        let cap = 5;
        let mut lru = Lru::new(cap);
        let mut model: Vec<(u32, u32)> = Vec::new(); // front = most recent
        let mut x = 12_345u32;
        for step in 0..5_000u32 {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let key = (x >> 16) % 9;
            if x & 1 == 0 {
                let got = lru.get(&key);
                let want = model.iter().position(|e| e.0 == key).map(|p| {
                    let e = model.remove(p);
                    model.insert(0, e);
                    e.1
                });
                assert_eq!(got, want, "step {step}");
            } else {
                lru.insert(key, step);
                if let Some(p) = model.iter().position(|e| e.0 == key) {
                    model.remove(p);
                }
                model.insert(0, (key, step));
                model.truncate(cap);
            }
        }
    }
}
