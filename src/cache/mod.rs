//! Local cache module
//!
//! Provides disk-based caching of decrypted chunks for fast local access.
//! Implements LRU eviction and prefetching.

mod lru;

pub use lru::LruCache;

use crate::config::CacheConfig;
use crate::error::{Error, Result};
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, info};

/// Disk-based chunk cache with LRU eviction
pub struct ChunkCache {
    /// Cache directory
    cache_dir: PathBuf,
    /// Maximum cache size in bytes
    max_size: u64,
    /// Current cache size
    current_size: AtomicU64,
    /// LRU tracking
    lru: RwLock<LruCache<String>>,
    /// Chunk sizes (for accurate size tracking)
    sizes: RwLock<HashMap<String, u64>>,
    /// Prefetch queue
    prefetch_queue: RwLock<VecDeque<String>>,
    /// Prefetch enabled
    prefetch_enabled: bool,
}

impl ChunkCache {
    /// Create a new chunk cache
    pub fn new(config: &CacheConfig) -> Result<Self> {
        // Ensure cache directory exists
        fs::create_dir_all(&config.cache_dir)?;

        let cache = ChunkCache {
            cache_dir: config.cache_dir.clone(),
            max_size: config.max_size,
            current_size: AtomicU64::new(0),
            lru: RwLock::new(LruCache::new()),
            sizes: RwLock::new(HashMap::new()),
            prefetch_queue: RwLock::new(VecDeque::new()),
            prefetch_enabled: config.prefetch_enabled,
        };

        // Scan existing cache
        cache.scan_cache()?;

        info!(
            "Cache initialized: {} bytes used of {} max",
            cache.current_size.load(Ordering::Relaxed),
            cache.max_size
        );

        Ok(cache)
    }

    /// Scan existing cache on startup
    fn scan_cache(&self) -> Result<()> {
        let mut total_size = 0u64;
        let mut lru = self.lru.write();
        let mut sizes = self.sizes.write();

        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            let size = metadata.len();
                            total_size += size;
                            lru.insert(name.to_string());
                            sizes.insert(name.to_string(), size);
                        }
                    }
                }
            }
        }

        self.current_size.store(total_size, Ordering::SeqCst);
        Ok(())
    }

    /// Get the path for a chunk
    fn chunk_path(&self, chunk_id: &str) -> PathBuf {
        self.cache_dir.join(chunk_id)
    }

    /// Check if a chunk is in cache
    pub fn contains(&self, chunk_id: &str) -> bool {
        self.chunk_path(chunk_id).exists()
    }

    /// Get a chunk from cache
    pub fn get(&self, chunk_id: &str) -> Result<Option<Vec<u8>>> {
        let path = self.chunk_path(chunk_id);

        if !path.exists() {
            return Ok(None);
        }

        // Update LRU
        self.lru.write().touch(&chunk_id.to_string());

        // Read file
        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        debug!("Cache hit: {} ({} bytes)", chunk_id, data.len());
        Ok(Some(data))
    }

    /// Put a chunk in cache
    pub fn put(&self, chunk_id: &str, data: &[u8]) -> Result<()> {
        let size = data.len() as u64;

        // Evict if necessary
        self.ensure_space(size)?;

        let path = self.chunk_path(chunk_id);

        // Write file
        let mut file = File::create(&path)?;
        file.write_all(data)?;
        file.sync_all()?;

        // Update tracking
        self.lru.write().insert(chunk_id.to_string());
        self.sizes.write().insert(chunk_id.to_string(), size);
        self.current_size.fetch_add(size, Ordering::SeqCst);

        debug!("Cached: {} ({} bytes)", chunk_id, size);
        Ok(())
    }

    /// Remove a chunk from cache
    pub fn remove(&self, chunk_id: &str) -> Result<()> {
        let path = self.chunk_path(chunk_id);

        if path.exists() {
            if let Some(size) = self.sizes.write().remove(chunk_id) {
                self.current_size.fetch_sub(size, Ordering::SeqCst);
            }
            self.lru.write().remove(&chunk_id.to_string());
            fs::remove_file(&path)?;
            debug!("Removed from cache: {}", chunk_id);
        }

        Ok(())
    }

    /// Ensure we have space for new data
    fn ensure_space(&self, needed: u64) -> Result<()> {
        let mut current = self.current_size.load(Ordering::SeqCst);

        while current + needed > self.max_size {
            // Evict oldest
            let to_evict = {
                let mut lru = self.lru.write();
                lru.pop_oldest()
            };

            match to_evict {
                Some(chunk_id) => {
                    let size = self.sizes.write().remove(&chunk_id).unwrap_or(0);
                    let path = self.chunk_path(&chunk_id);
                    if path.exists() {
                        fs::remove_file(&path)?;
                    }
                    self.current_size.fetch_sub(size, Ordering::SeqCst);
                    current = self.current_size.load(Ordering::SeqCst);
                    debug!("Evicted from cache: {} ({} bytes)", chunk_id, size);
                }
                None => {
                    // Cache is empty but still can't fit
                    if needed > self.max_size {
                        return Err(Error::CacheFull);
                    }
                    break;
                }
            }
        }

        Ok(())
    }

    /// Queue chunks for prefetching
    pub fn queue_prefetch(&self, chunk_ids: Vec<String>) {
        if !self.prefetch_enabled {
            return;
        }

        let mut queue = self.prefetch_queue.write();
        for id in chunk_ids {
            if !self.contains(&id) && !queue.contains(&id) {
                queue.push_back(id);
            }
        }
    }

    /// Get next chunk to prefetch
    pub fn next_prefetch(&self) -> Option<String> {
        self.prefetch_queue.write().pop_front()
    }

    /// Get current cache size
    pub fn size(&self) -> u64 {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Get number of cached chunks
    pub fn count(&self) -> usize {
        self.sizes.read().len()
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            current_size: self.size(),
            max_size: self.max_size,
            chunk_count: self.count(),
            prefetch_queue_len: self.prefetch_queue.read().len(),
        }
    }

    /// Clear the entire cache
    pub fn clear(&self) -> Result<()> {
        // Remove all files
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if entry.metadata().map(|m| m.is_file()).unwrap_or(false) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        // Reset tracking
        self.lru.write().clear();
        self.sizes.write().clear();
        self.current_size.store(0, Ordering::SeqCst);

        info!("Cache cleared");
        Ok(())
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub current_size: u64,
    pub max_size: u64,
    pub chunk_count: usize,
    pub prefetch_queue_len: usize,
}

impl CacheStats {
    /// Get cache utilization as a percentage
    pub fn utilization(&self) -> f64 {
        if self.max_size == 0 {
            0.0
        } else {
            (self.current_size as f64 / self.max_size as f64) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn test_config(dir: &Path) -> CacheConfig {
        CacheConfig {
            max_size: 10000,
            cache_dir: dir.to_path_buf(),
            prefetch_enabled: true,
            prefetch_count: 3,
            eviction_policy: crate::config::EvictionPolicy::Lru,
        }
    }

    #[test]
    fn test_cache_put_get() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let cache = ChunkCache::new(&config).unwrap();

        cache.put("chunk1", b"hello world").unwrap();
        assert!(cache.contains("chunk1"));

        let data = cache.get("chunk1").unwrap().unwrap();
        assert_eq!(data, b"hello world");
    }

    #[test]
    fn test_cache_miss() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let cache = ChunkCache::new(&config).unwrap();

        assert!(!cache.contains("nonexistent"));
        assert!(cache.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_cache_remove() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let cache = ChunkCache::new(&config).unwrap();

        cache.put("chunk1", b"data").unwrap();
        assert!(cache.contains("chunk1"));

        cache.remove("chunk1").unwrap();
        assert!(!cache.contains("chunk1"));
    }

    #[test]
    fn test_cache_eviction() {
        let temp = TempDir::new().unwrap();
        let mut config = test_config(temp.path());
        config.max_size = 100; // Very small cache

        let cache = ChunkCache::new(&config).unwrap();

        // Fill cache
        cache.put("chunk1", &[0u8; 40]).unwrap();
        cache.put("chunk2", &[0u8; 40]).unwrap();

        // This should evict chunk1
        cache.put("chunk3", &[0u8; 40]).unwrap();

        assert!(!cache.contains("chunk1"));
        assert!(cache.contains("chunk2"));
        assert!(cache.contains("chunk3"));
    }

    #[test]
    fn test_cache_lru_ordering() {
        let temp = TempDir::new().unwrap();
        let mut config = test_config(temp.path());
        config.max_size = 100;

        let cache = ChunkCache::new(&config).unwrap();

        cache.put("chunk1", &[0u8; 30]).unwrap();
        cache.put("chunk2", &[0u8; 30]).unwrap();

        // Access chunk1 to make it more recent
        cache.get("chunk1").unwrap();

        // This should evict chunk2 (least recently used)
        cache.put("chunk3", &[0u8; 50]).unwrap();

        assert!(cache.contains("chunk1"));
        assert!(!cache.contains("chunk2"));
        assert!(cache.contains("chunk3"));
    }

    #[test]
    fn test_prefetch_queue() {
        let temp = TempDir::new().unwrap();
        let config = test_config(temp.path());
        let cache = ChunkCache::new(&config).unwrap();

        cache.queue_prefetch(vec!["a".to_string(), "b".to_string(), "c".to_string()]);

        assert_eq!(cache.next_prefetch(), Some("a".to_string()));
        assert_eq!(cache.next_prefetch(), Some("b".to_string()));
        assert_eq!(cache.next_prefetch(), Some("c".to_string()));
        assert_eq!(cache.next_prefetch(), None);
    }
}
