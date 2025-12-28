//! Whiteout tracking for overlay filesystem
//!
//! Tracks deleted files and opaque directories to hide lower layer entries.

use crate::error::Result;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Prefix for whiteout entries in sled (reserved for future prefix-based keys)
#[allow(dead_code)]
const WHITEOUT_PREFIX: &[u8] = b"wo:";
/// Prefix for opaque directory markers (reserved for future prefix-based keys)
#[allow(dead_code)]
const OPAQUE_PREFIX: &[u8] = b"op:";

/// Tracks deleted files and opaque directories
pub struct WhiteoutStore {
    /// Sled database for persistence
    db: sled::Db,
    /// Whiteout entries tree (deleted files)
    whiteouts: sled::Tree,
    /// Opaque directories tree
    opaque_dirs: sled::Tree,
    /// In-memory cache for fast lookups
    cache: RwLock<WhiteoutCache>,
}

/// In-memory cache for whiteout lookups
struct WhiteoutCache {
    /// Set of whiteout paths (normalized, relative to overlay root)
    whiteouts: HashSet<PathBuf>,
    /// Set of opaque directory paths
    opaque_dirs: HashSet<PathBuf>,
    /// Whether cache is fully loaded
    loaded: bool,
}

impl WhiteoutStore {
    /// Create/open whiteout store at the given path
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = sled::open(path.as_ref())?;

        let whiteouts = db.open_tree("whiteouts")?;

        let opaque_dirs = db.open_tree("opaque_dirs")?;

        let store = Self {
            db,
            whiteouts,
            opaque_dirs,
            cache: RwLock::new(WhiteoutCache {
                whiteouts: HashSet::new(),
                opaque_dirs: HashSet::new(),
                loaded: false,
            }),
        };

        store.load_cache()?;
        Ok(store)
    }

    /// Load all whiteouts into cache
    fn load_cache(&self) -> Result<()> {
        let mut cache = self.cache.write();

        for entry in self.whiteouts.iter() {
            let (key, _) = entry?;
            if let Ok(path_str) = std::str::from_utf8(&key) {
                cache.whiteouts.insert(PathBuf::from(path_str));
            }
        }

        for entry in self.opaque_dirs.iter() {
            let (key, _) = entry?;
            if let Ok(path_str) = std::str::from_utf8(&key) {
                cache.opaque_dirs.insert(PathBuf::from(path_str));
            }
        }

        cache.loaded = true;
        debug!(
            "Loaded {} whiteouts and {} opaque dirs into cache",
            cache.whiteouts.len(),
            cache.opaque_dirs.len()
        );
        Ok(())
    }

    /// Check if a path is whited-out (deleted)
    pub fn is_whiteout(&self, path: &Path) -> bool {
        let cache = self.cache.read();
        cache.whiteouts.contains(path)
    }

    /// Check if a path is under an opaque directory
    pub fn is_under_opaque(&self, path: &Path) -> bool {
        let cache = self.cache.read();
        for ancestor in path.ancestors().skip(1) {
            if cache.opaque_dirs.contains(ancestor) {
                return true;
            }
        }
        false
    }

    /// Add a whiteout (mark as deleted)
    pub fn add_whiteout(&self, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        self.whiteouts.insert(path_str.as_bytes(), b"1")?;

        let mut cache = self.cache.write();
        cache.whiteouts.insert(path.to_path_buf());
        debug!("Added whiteout for: {:?}", path);
        Ok(())
    }

    /// Remove a whiteout (file re-created)
    pub fn remove_whiteout(&self, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        self.whiteouts.remove(path_str.as_bytes())?;

        let mut cache = self.cache.write();
        cache.whiteouts.remove(path);
        debug!("Removed whiteout for: {:?}", path);
        Ok(())
    }

    /// Mark directory as opaque (hide lower contents)
    pub fn mark_opaque(&self, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        self.opaque_dirs.insert(path_str.as_bytes(), b"1")?;

        let mut cache = self.cache.write();
        cache.opaque_dirs.insert(path.to_path_buf());
        debug!("Marked directory as opaque: {:?}", path);
        Ok(())
    }

    /// Unmark directory as opaque
    pub fn unmark_opaque(&self, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        self.opaque_dirs.remove(path_str.as_bytes())?;

        let mut cache = self.cache.write();
        cache.opaque_dirs.remove(path);
        Ok(())
    }

    /// Check if directory is opaque
    pub fn is_opaque(&self, path: &Path) -> bool {
        let cache = self.cache.read();
        cache.opaque_dirs.contains(path)
    }

    /// Get all whiteouts under a directory (for readdir filtering)
    pub fn whiteouts_in_dir(&self, dir: &Path) -> HashSet<OsString> {
        let cache = self.cache.read();
        let mut result = HashSet::new();

        for whiteout_path in &cache.whiteouts {
            if let Some(parent) = whiteout_path.parent() {
                if parent == dir {
                    if let Some(name) = whiteout_path.file_name() {
                        result.insert(name.to_os_string());
                    }
                }
            }
        }

        result
    }

    /// Clear all whiteouts (for sync operations)
    pub fn clear(&self) -> Result<()> {
        self.whiteouts.clear()?;
        self.opaque_dirs.clear()?;

        let mut cache = self.cache.write();
        cache.whiteouts.clear();
        cache.opaque_dirs.clear();

        debug!("Cleared all whiteouts and opaque directories");
        Ok(())
    }

    /// Flush to disk
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_whiteout_store() {
        let dir = tempdir().unwrap();
        let store = WhiteoutStore::open(dir.path().join("whiteout.db")).unwrap();

        let path = PathBuf::from("test/file.txt");

        assert!(!store.is_whiteout(&path));
        store.add_whiteout(&path).unwrap();
        assert!(store.is_whiteout(&path));
        store.remove_whiteout(&path).unwrap();
        assert!(!store.is_whiteout(&path));
    }

    #[test]
    fn test_opaque_dirs() {
        let dir = tempdir().unwrap();
        let store = WhiteoutStore::open(dir.path().join("whiteout.db")).unwrap();

        let dir_path = PathBuf::from("test/dir");
        let child_path = PathBuf::from("test/dir/child.txt");

        assert!(!store.is_opaque(&dir_path));
        assert!(!store.is_under_opaque(&child_path));

        store.mark_opaque(&dir_path).unwrap();
        assert!(store.is_opaque(&dir_path));
        assert!(store.is_under_opaque(&child_path));
    }
}
