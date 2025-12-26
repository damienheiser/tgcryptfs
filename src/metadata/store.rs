//! Sled-based metadata store
//!
//! All metadata is encrypted before storage. The database contains
//! encrypted blobs that can only be read with the correct key.

use crate::crypto::{decrypt, encrypt, EncryptedData, KEY_SIZE};
use crate::error::{Error, Result};
use crate::metadata::Inode;
use parking_lot::RwLock;
use sled::{Db, Tree};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{debug, info};

/// Key prefixes for different data types
#[allow(dead_code)]
const INODE_PREFIX: &[u8] = b"ino:";
#[allow(dead_code)]
const PARENT_PREFIX: &[u8] = b"par:";
#[allow(dead_code)]
const CHUNK_PREFIX: &[u8] = b"chk:";
#[allow(dead_code)]
const META_PREFIX: &[u8] = b"meta:";

/// Encrypted metadata store using sled
pub struct MetadataStore {
    /// Sled database
    db: Db,
    /// Inodes tree
    inodes: Tree,
    /// Parent-name index tree
    parent_index: Tree,
    /// Chunk references tree
    chunks: Tree,
    /// General metadata tree
    metadata: Tree,
    /// Encryption key for metadata
    key: [u8; KEY_SIZE],
    /// Next available inode number
    next_ino: AtomicU64,
    /// In-memory inode cache
    cache: RwLock<HashMap<u64, Inode>>,
    /// Optional namespace prefix for storage keys
    namespace_prefix: Option<String>,
}

impl MetadataStore {
    /// Open or create a metadata store
    pub fn open<P: AsRef<Path>>(path: P, key: [u8; KEY_SIZE]) -> Result<Self> {
        Self::open_with_namespace(path, key, None)
    }

    /// Open or create a metadata store with a namespace prefix
    pub fn open_with_namespace<P: AsRef<Path>>(
        path: P,
        key: [u8; KEY_SIZE],
        namespace_prefix: Option<String>,
    ) -> Result<Self> {
        let db = sled::open(path.as_ref())?;

        // Use namespace-prefixed tree names if namespace is provided
        let (inodes_name, parent_name, chunks_name, metadata_name) = match &namespace_prefix {
            Some(prefix) => (
                format!("{}:inodes", prefix),
                format!("{}:parent_index", prefix),
                format!("{}:chunks", prefix),
                format!("{}:metadata", prefix),
            ),
            None => (
                "inodes".to_string(),
                "parent_index".to_string(),
                "chunks".to_string(),
                "metadata".to_string(),
            ),
        };

        let inodes = db.open_tree(&inodes_name)?;
        let parent_index = db.open_tree(&parent_name)?;
        let chunks = db.open_tree(&chunks_name)?;
        let metadata = db.open_tree(&metadata_name)?;

        // Get max inode number
        let max_ino = inodes
            .iter()
            .keys()
            .filter_map(|r| r.ok())
            .filter_map(|k| {
                if k.len() >= 8 {
                    Some(u64::from_be_bytes(k[..8].try_into().unwrap()))
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);

        let store = MetadataStore {
            db,
            inodes,
            parent_index,
            chunks,
            metadata,
            key,
            next_ino: AtomicU64::new(max_ino + 1),
            cache: RwLock::new(HashMap::new()),
            namespace_prefix,
        };

        // Initialize root if needed
        if max_ino == 0 {
            store.init_root()?;
        }

        info!(
            "Metadata store opened, max inode: {}, namespace: {:?}",
            max_ino, store.namespace_prefix
        );
        Ok(store)
    }

    /// Create an in-memory store (for testing)
    pub fn in_memory(key: [u8; KEY_SIZE]) -> Result<Self> {
        Self::in_memory_with_namespace(key, None)
    }

    /// Create an in-memory store with namespace prefix (for testing)
    pub fn in_memory_with_namespace(
        key: [u8; KEY_SIZE],
        namespace_prefix: Option<String>,
    ) -> Result<Self> {
        let db = sled::Config::new().temporary(true).open()?;

        // Use namespace-prefixed tree names if namespace is provided
        let (inodes_name, parent_name, chunks_name, metadata_name) = match &namespace_prefix {
            Some(prefix) => (
                format!("{}:inodes", prefix),
                format!("{}:parent_index", prefix),
                format!("{}:chunks", prefix),
                format!("{}:metadata", prefix),
            ),
            None => (
                "inodes".to_string(),
                "parent_index".to_string(),
                "chunks".to_string(),
                "metadata".to_string(),
            ),
        };

        let inodes = db.open_tree(&inodes_name)?;
        let parent_index = db.open_tree(&parent_name)?;
        let chunks = db.open_tree(&chunks_name)?;
        let metadata = db.open_tree(&metadata_name)?;

        let store = MetadataStore {
            db,
            inodes,
            parent_index,
            chunks,
            metadata,
            key,
            next_ino: AtomicU64::new(1),
            cache: RwLock::new(HashMap::new()),
            namespace_prefix,
        };

        store.init_root()?;
        Ok(store)
    }

    /// Initialize the root inode
    fn init_root(&self) -> Result<()> {
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let root = Inode::root(uid, gid, 0o755);
        self.save_inode(&root)?;
        self.next_ino.store(2, Ordering::SeqCst);
        info!("Root inode initialized");
        Ok(())
    }

    /// Allocate a new inode number
    pub fn alloc_ino(&self) -> u64 {
        self.next_ino.fetch_add(1, Ordering::SeqCst)
    }

    /// Create inode key from ino
    fn inode_key(ino: u64) -> [u8; 8] {
        ino.to_be_bytes()
    }

    /// Create parent-name index key
    fn parent_name_key(parent: u64, name: &str) -> Vec<u8> {
        let mut key = Vec::with_capacity(8 + name.len());
        key.extend_from_slice(&parent.to_be_bytes());
        key.extend_from_slice(name.as_bytes());
        key
    }

    /// Encrypt an inode for storage
    fn encrypt_inode(&self, inode: &Inode) -> Result<Vec<u8>> {
        let data = bincode::serialize(inode)?;
        let encrypted = encrypt(&self.key, &data, &[])?;
        Ok(encrypted.to_bytes())
    }

    /// Decrypt an inode from storage
    fn decrypt_inode(&self, data: &[u8]) -> Result<Inode> {
        let encrypted = EncryptedData::from_bytes(data)?;
        let decrypted = decrypt(&self.key, &encrypted, &[])?;
        let inode: Inode = bincode::deserialize(&decrypted)?;
        Ok(inode)
    }

    /// Save an inode to the database
    pub fn save_inode(&self, inode: &Inode) -> Result<()> {
        let encrypted = self.encrypt_inode(inode)?;
        let key = Self::inode_key(inode.ino);

        // Save inode data
        self.inodes.insert(key, encrypted)?;

        // Update parent-name index
        let parent_key = Self::parent_name_key(inode.parent, &inode.name);
        self.parent_index.insert(parent_key, &key[..])?;

        // Update cache
        self.cache.write().insert(inode.ino, inode.clone());

        debug!("Saved inode {} ({})", inode.ino, inode.name);
        Ok(())
    }

    /// Get an inode by number
    pub fn get_inode(&self, ino: u64) -> Result<Option<Inode>> {
        // Check cache first
        if let Some(inode) = self.cache.read().get(&ino) {
            return Ok(Some(inode.clone()));
        }

        let key = Self::inode_key(ino);
        match self.inodes.get(key)? {
            Some(data) => {
                let inode = self.decrypt_inode(&data)?;
                self.cache.write().insert(ino, inode.clone());
                Ok(Some(inode))
            }
            None => Ok(None),
        }
    }

    /// Get an inode, returning an error if not found
    pub fn get_inode_required(&self, ino: u64) -> Result<Inode> {
        self.get_inode(ino)?
            .ok_or_else(|| Error::InodeNotFound(ino))
    }

    /// Lookup a child by name in a directory
    pub fn lookup(&self, parent: u64, name: &str) -> Result<Option<Inode>> {
        let parent_key = Self::parent_name_key(parent, name);

        match self.parent_index.get(parent_key)? {
            Some(ino_bytes) => {
                if ino_bytes.len() >= 8 {
                    let ino = u64::from_be_bytes(ino_bytes[..8].try_into().unwrap());
                    self.get_inode(ino)
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Delete an inode
    pub fn delete_inode(&self, ino: u64) -> Result<()> {
        // Get the inode first to remove from parent index
        if let Some(inode) = self.get_inode(ino)? {
            let parent_key = Self::parent_name_key(inode.parent, &inode.name);
            self.parent_index.remove(parent_key)?;
        }

        let key = Self::inode_key(ino);
        self.inodes.remove(key)?;
        self.cache.write().remove(&ino);
        debug!("Deleted inode {}", ino);
        Ok(())
    }

    /// Get all children of a directory
    pub fn get_children(&self, parent: u64) -> Result<Vec<Inode>> {
        let prefix = parent.to_be_bytes();
        let mut children = Vec::new();

        for result in self.parent_index.scan_prefix(&prefix) {
            let (_, ino_bytes) = result?;
            if ino_bytes.len() >= 8 {
                let ino = u64::from_be_bytes(ino_bytes[..8].try_into().unwrap());
                if let Some(inode) = self.get_inode(ino)? {
                    // Make sure it's actually a child (not just prefix match)
                    // Also exclude the parent itself (root inode is its own parent)
                    if inode.parent == parent && inode.ino != parent {
                        children.push(inode);
                    }
                }
            }
        }

        Ok(children)
    }

    /// Save a chunk reference
    pub fn save_chunk_ref(&self, chunk_id: &str, message_id: i32) -> Result<()> {
        let key = chunk_id.as_bytes();

        // Get existing ref count or start at 0
        let ref_count = match self.chunks.get(key)? {
            Some(data) if data.len() >= 8 => {
                let count = u32::from_be_bytes(data[4..8].try_into().unwrap());
                count + 1
            }
            _ => 1,
        };

        // Store: message_id (4 bytes) + ref_count (4 bytes)
        let mut value = Vec::with_capacity(8);
        value.extend_from_slice(&message_id.to_be_bytes());
        value.extend_from_slice(&ref_count.to_be_bytes());

        self.chunks.insert(key, value)?;
        Ok(())
    }

    /// Get a chunk reference
    pub fn get_chunk_ref(&self, chunk_id: &str) -> Result<Option<i32>> {
        let key = chunk_id.as_bytes();

        match self.chunks.get(key)? {
            Some(data) if data.len() >= 4 => {
                let msg_id = i32::from_be_bytes(data[..4].try_into().unwrap());
                Ok(Some(msg_id))
            }
            _ => Ok(None),
        }
    }

    /// Decrement chunk reference count
    pub fn decrement_chunk_ref(&self, chunk_id: &str) -> Result<Option<i32>> {
        let key = chunk_id.as_bytes();

        match self.chunks.get(key)? {
            Some(data) if data.len() >= 8 => {
                let msg_id = i32::from_be_bytes(data[..4].try_into().unwrap());
                let ref_count = u32::from_be_bytes(data[4..8].try_into().unwrap());

                if ref_count <= 1 {
                    // Delete the reference
                    self.chunks.remove(key)?;
                    Ok(Some(msg_id)) // Return message_id to delete from Telegram
                } else {
                    // Decrement count
                    let mut value = Vec::with_capacity(8);
                    value.extend_from_slice(&msg_id.to_be_bytes());
                    value.extend_from_slice(&(ref_count - 1).to_be_bytes());
                    self.chunks.insert(key, value)?;
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Save general metadata
    pub fn save_metadata(&self, key: &str, value: &[u8]) -> Result<()> {
        let encrypted = encrypt(&self.key, value, &[])?;
        self.metadata.insert(key.as_bytes(), encrypted.to_bytes())?;
        Ok(())
    }

    /// Get general metadata
    pub fn get_metadata(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.metadata.get(key.as_bytes())? {
            Some(data) => {
                let encrypted = EncryptedData::from_bytes(&data)?;
                let decrypted = decrypt(&self.key, &encrypted, &[])?;
                Ok(Some(decrypted))
            }
            None => Ok(None),
        }
    }

    /// Get filesystem statistics
    pub fn get_stats(&self) -> Result<FsStats> {
        let inode_count = self.inodes.len() as u64;
        let chunk_count = self.chunks.len() as u64;

        Ok(FsStats {
            inode_count,
            chunk_count,
        })
    }

    /// Clear the cache
    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }

    /// Flush to disk
    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }

    /// Get the namespace prefix
    pub fn namespace_prefix(&self) -> Option<&str> {
        self.namespace_prefix.as_deref()
    }

    /// Check if this store is namespaced
    pub fn is_namespaced(&self) -> bool {
        self.namespace_prefix.is_some()
    }
}

/// Filesystem statistics
#[derive(Debug, Clone)]
pub struct FsStats {
    pub inode_count: u64,
    pub chunk_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    fn test_key() -> [u8; KEY_SIZE] {
        let mut key = [0u8; KEY_SIZE];
        rand::thread_rng().fill_bytes(&mut key);
        key
    }

    #[test]
    fn test_create_store() {
        let key = test_key();
        let store = MetadataStore::in_memory(key).unwrap();

        // Root should exist
        let root = store.get_inode(1).unwrap().unwrap();
        assert!(root.is_dir());
    }

    #[test]
    fn test_save_and_get_inode() {
        let key = test_key();
        let store = MetadataStore::in_memory(key).unwrap();

        let file = Inode::new_file(2, 1, "test.txt".to_string(), 1000, 1000, 0o644);
        store.save_inode(&file).unwrap();

        let retrieved = store.get_inode(2).unwrap().unwrap();
        assert_eq!(retrieved.name, "test.txt");
        assert!(retrieved.is_file());
    }

    #[test]
    fn test_lookup() {
        let key = test_key();
        let store = MetadataStore::in_memory(key).unwrap();

        let file = Inode::new_file(2, 1, "test.txt".to_string(), 1000, 1000, 0o644);
        store.save_inode(&file).unwrap();

        let found = store.lookup(1, "test.txt").unwrap().unwrap();
        assert_eq!(found.ino, 2);

        let not_found = store.lookup(1, "nonexistent").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_get_children() {
        let key = test_key();
        let store = MetadataStore::in_memory(key).unwrap();

        // Create files in root
        for i in 2..5 {
            let file = Inode::new_file(i, 1, format!("file{}.txt", i), 1000, 1000, 0o644);
            store.save_inode(&file).unwrap();
        }

        let children = store.get_children(1).unwrap();
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn test_delete_inode() {
        let key = test_key();
        let store = MetadataStore::in_memory(key).unwrap();

        let file = Inode::new_file(2, 1, "test.txt".to_string(), 1000, 1000, 0o644);
        store.save_inode(&file).unwrap();

        store.delete_inode(2).unwrap();

        let result = store.get_inode(2).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_chunk_refs() {
        let key = test_key();
        let store = MetadataStore::in_memory(key).unwrap();

        store.save_chunk_ref("chunk1", 100).unwrap();
        store.save_chunk_ref("chunk1", 100).unwrap(); // Add reference

        assert_eq!(store.get_chunk_ref("chunk1").unwrap(), Some(100));

        // First decrement shouldn't delete
        assert!(store.decrement_chunk_ref("chunk1").unwrap().is_none());

        // Second decrement should return message_id for deletion
        assert_eq!(store.decrement_chunk_ref("chunk1").unwrap(), Some(100));

        // Should be gone now
        assert!(store.get_chunk_ref("chunk1").unwrap().is_none());
    }

    #[test]
    fn test_metadata() {
        let key = test_key();
        let store = MetadataStore::in_memory(key).unwrap();

        store.save_metadata("test_key", b"test_value").unwrap();

        let value = store.get_metadata("test_key").unwrap().unwrap();
        assert_eq!(value, b"test_value");
    }
}
