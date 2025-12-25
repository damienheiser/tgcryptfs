//! LRU (Least Recently Used) tracking

use std::collections::{HashMap, VecDeque};

/// Simple LRU tracker
pub struct LruCache<K: Clone + Eq + std::hash::Hash> {
    /// Order of access (front = oldest)
    order: VecDeque<K>,
    /// Position lookup for O(1) removal
    positions: HashMap<K, usize>,
    /// Generation counter for lazy cleanup
    generation: usize,
}

impl<K: Clone + Eq + std::hash::Hash> LruCache<K> {
    /// Create a new LRU cache
    pub fn new() -> Self {
        LruCache {
            order: VecDeque::new(),
            positions: HashMap::new(),
            generation: 0,
        }
    }

    /// Insert a new item (as most recently used)
    pub fn insert(&mut self, key: K) {
        self.generation += 1;
        self.positions.insert(key.clone(), self.generation);
        self.order.push_back(key);
    }

    /// Touch an item (mark as recently used)
    pub fn touch(&mut self, key: &K) {
        if self.positions.contains_key(key) {
            self.generation += 1;
            self.positions.insert(key.clone(), self.generation);
            self.order.push_back(key.clone());
        }
    }

    /// Remove an item
    pub fn remove(&mut self, key: &K) {
        self.positions.remove(key);
        // Lazy removal - will be skipped when popping
    }

    /// Pop the oldest item
    pub fn pop_oldest(&mut self) -> Option<K> {
        while let Some(key) = self.order.pop_front() {
            // Check if this entry is still valid (not updated since)
            if let Some(&gen) = self.positions.get(&key) {
                // Check if there's a newer entry for this key
                let is_newest = self.order.iter().all(|k| {
                    k != &key || self.positions.get(k).map(|&g| g <= gen).unwrap_or(true)
                });

                if is_newest && self.positions.remove(&key).is_some() {
                    return Some(key);
                }
            }
        }
        None
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Get count of tracked items
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// Clear all items
    pub fn clear(&mut self) {
        self.order.clear();
        self.positions.clear();
        self.generation = 0;
    }

    /// Compact the internal structures (remove stale entries)
    pub fn compact(&mut self) {
        // Rebuild order from positions
        let mut items: Vec<_> = self.positions.iter().map(|(k, &g)| (k.clone(), g)).collect();
        items.sort_by_key(|(_, g)| *g);

        self.order.clear();
        for (key, _) in items {
            self.order.push_back(key);
        }
    }
}

impl<K: Clone + Eq + std::hash::Hash> Default for LruCache<K> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_pop() {
        let mut lru = LruCache::new();

        lru.insert("a");
        lru.insert("b");
        lru.insert("c");

        assert_eq!(lru.len(), 3);
        assert_eq!(lru.pop_oldest(), Some("a"));
        assert_eq!(lru.pop_oldest(), Some("b"));
        assert_eq!(lru.pop_oldest(), Some("c"));
        assert_eq!(lru.pop_oldest(), None);
    }

    #[test]
    fn test_touch_updates_order() {
        let mut lru = LruCache::new();

        lru.insert("a");
        lru.insert("b");
        lru.insert("c");

        // Touch 'a' to make it most recent
        lru.touch(&"a");

        assert_eq!(lru.pop_oldest(), Some("b"));
        assert_eq!(lru.pop_oldest(), Some("c"));
        assert_eq!(lru.pop_oldest(), Some("a"));
    }

    #[test]
    fn test_remove() {
        let mut lru = LruCache::new();

        lru.insert("a");
        lru.insert("b");
        lru.insert("c");

        lru.remove(&"b");

        assert_eq!(lru.len(), 2);
        assert_eq!(lru.pop_oldest(), Some("a"));
        assert_eq!(lru.pop_oldest(), Some("c"));
    }

    #[test]
    fn test_clear() {
        let mut lru = LruCache::new();

        lru.insert("a");
        lru.insert("b");

        lru.clear();

        assert!(lru.is_empty());
        assert_eq!(lru.pop_oldest(), None);
    }
}
