//! Snapshot implementation
//!
//! A snapshot captures the state of all inodes at a point in time.
//! Since chunk data is immutable and content-addressed, snapshots
//! only need to store inode metadata.

use crate::crypto::{decrypt, encrypt, EncryptedData, KEY_SIZE};
use crate::error::{Error, Result};
use crate::metadata::Inode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A filesystem snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique snapshot ID
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Creation timestamp
    pub created: DateTime<Utc>,
    /// Optional description
    pub description: Option<String>,
    /// Snapshot of all inodes (ino -> serialized inode)
    pub inodes: HashMap<u64, Vec<u8>>,
    /// Root inode number
    pub root_ino: u64,
}

impl Snapshot {
    /// Create a new snapshot
    pub fn new(name: String, description: Option<String>) -> Self {
        Snapshot {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            created: Utc::now(),
            description,
            inodes: HashMap::new(),
            root_ino: 1,
        }
    }

    /// Add an inode to the snapshot
    pub fn add_inode(&mut self, inode: &Inode) -> Result<()> {
        let serialized = bincode::serialize(inode)?;
        self.inodes.insert(inode.ino, serialized);
        Ok(())
    }

    /// Get an inode from the snapshot
    pub fn get_inode(&self, ino: u64) -> Result<Option<Inode>> {
        match self.inodes.get(&ino) {
            Some(data) => {
                let inode: Inode = bincode::deserialize(data)?;
                Ok(Some(inode))
            }
            None => Ok(None),
        }
    }

    /// Get all inodes in the snapshot
    pub fn all_inodes(&self) -> Result<Vec<Inode>> {
        let mut inodes = Vec::with_capacity(self.inodes.len());
        for data in self.inodes.values() {
            let inode: Inode = bincode::deserialize(data)?;
            inodes.push(inode);
        }
        Ok(inodes)
    }

    /// Get the number of inodes in the snapshot
    pub fn inode_count(&self) -> usize {
        self.inodes.len()
    }

    /// Serialize the snapshot for storage
    pub fn serialize(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(|e| Error::Serialization(e.to_string()))
    }

    /// Deserialize a snapshot
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        bincode::deserialize(data).map_err(|e| Error::Deserialization(e.to_string()))
    }
}

/// Manages snapshots
pub struct SnapshotManager {
    /// Encryption key
    key: [u8; KEY_SIZE],
    /// Maximum snapshots to keep
    max_snapshots: usize,
    /// Loaded snapshots
    snapshots: Vec<Snapshot>,
}

impl SnapshotManager {
    /// Create a new snapshot manager
    pub fn new(key: [u8; KEY_SIZE], max_snapshots: usize) -> Self {
        SnapshotManager {
            key,
            max_snapshots,
            snapshots: Vec::new(),
        }
    }

    /// Create a snapshot from current metadata
    pub fn create_snapshot<F>(
        &mut self,
        name: String,
        description: Option<String>,
        iter_inodes: F,
    ) -> Result<&Snapshot>
    where
        F: FnOnce() -> Result<Vec<Inode>>,
    {
        let mut snapshot = Snapshot::new(name, description);

        // Collect all inodes
        let inodes = iter_inodes()?;
        for inode in inodes {
            snapshot.add_inode(&inode)?;
        }

        // Prune old snapshots if needed
        if self.max_snapshots > 0 && self.snapshots.len() >= self.max_snapshots {
            self.snapshots.remove(0);
        }

        self.snapshots.push(snapshot);
        Ok(self.snapshots.last().unwrap())
    }

    /// List all snapshots
    pub fn list(&self) -> &[Snapshot] {
        &self.snapshots
    }

    /// Get a snapshot by ID
    pub fn get(&self, id: &str) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.id == id)
    }

    /// Get a snapshot by name
    pub fn get_by_name(&self, name: &str) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.name == name)
    }

    /// Delete a snapshot
    pub fn delete(&mut self, id: &str) -> bool {
        if let Some(pos) = self.snapshots.iter().position(|s| s.id == id) {
            self.snapshots.remove(pos);
            true
        } else {
            false
        }
    }

    /// Encrypt and serialize all snapshots for storage
    pub fn export(&self) -> Result<Vec<u8>> {
        let data = bincode::serialize(&self.snapshots)?;
        let encrypted = encrypt(&self.key, &data, b"snapshots")?;
        Ok(encrypted.to_bytes())
    }

    /// Import snapshots from encrypted data
    pub fn import(&mut self, data: &[u8]) -> Result<()> {
        let encrypted = EncryptedData::from_bytes(data)?;
        let decrypted = decrypt(&self.key, &encrypted, b"snapshots")?;
        self.snapshots = bincode::deserialize(&decrypted)?;
        Ok(())
    }

    /// Get the latest snapshot
    pub fn latest(&self) -> Option<&Snapshot> {
        self.snapshots.last()
    }
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

    fn test_inode(ino: u64, name: &str) -> Inode {
        Inode::new_file(ino, 1, name.to_string(), 1000, 1000, 0o644)
    }

    #[test]
    fn test_snapshot_creation() {
        let mut snapshot = Snapshot::new("test".to_string(), Some("Test snapshot".to_string()));

        snapshot.add_inode(&test_inode(1, "file1.txt")).unwrap();
        snapshot.add_inode(&test_inode(2, "file2.txt")).unwrap();

        assert_eq!(snapshot.inode_count(), 2);
        assert!(snapshot.get_inode(1).unwrap().is_some());
        assert!(snapshot.get_inode(99).unwrap().is_none());
    }

    #[test]
    fn test_snapshot_serialization() {
        let mut snapshot = Snapshot::new("test".to_string(), None);
        snapshot.add_inode(&test_inode(1, "file.txt")).unwrap();

        let serialized = snapshot.serialize().unwrap();
        let restored = Snapshot::deserialize(&serialized).unwrap();

        assert_eq!(restored.id, snapshot.id);
        assert_eq!(restored.name, snapshot.name);
        assert_eq!(restored.inode_count(), 1);
    }

    #[test]
    fn test_snapshot_manager() {
        let key = test_key();
        let mut manager = SnapshotManager::new(key, 3);

        manager
            .create_snapshot("snap1".to_string(), None, || {
                Ok(vec![test_inode(1, "file1.txt")])
            })
            .unwrap();

        manager
            .create_snapshot("snap2".to_string(), None, || {
                Ok(vec![test_inode(1, "file1.txt"), test_inode(2, "file2.txt")])
            })
            .unwrap();

        assert_eq!(manager.list().len(), 2);
        assert!(manager.get_by_name("snap1").is_some());
        assert!(manager.get_by_name("snap2").is_some());
    }

    #[test]
    fn test_snapshot_limit() {
        let key = test_key();
        let mut manager = SnapshotManager::new(key, 2);

        for i in 1..=3 {
            manager
                .create_snapshot(format!("snap{}", i), None, || Ok(vec![]))
                .unwrap();
        }

        // Should only have 2 snapshots
        assert_eq!(manager.list().len(), 2);
        // Oldest should be removed
        assert!(manager.get_by_name("snap1").is_none());
        assert!(manager.get_by_name("snap2").is_some());
        assert!(manager.get_by_name("snap3").is_some());
    }

    #[test]
    fn test_export_import() {
        let key = test_key();
        let mut manager = SnapshotManager::new(key, 10);

        manager
            .create_snapshot("test".to_string(), None, || {
                Ok(vec![test_inode(1, "file.txt")])
            })
            .unwrap();

        let exported = manager.export().unwrap();

        let mut manager2 = SnapshotManager::new(key, 10);
        manager2.import(&exported).unwrap();

        assert_eq!(manager2.list().len(), 1);
        assert_eq!(manager2.list()[0].name, "test");
    }
}
