//! Master-Replica replication protocol
//!
//! This module implements a simple master-replica synchronization system where:
//! - One master node has write access
//! - Multiple replica nodes have read-only access
//! - The master periodically creates snapshots and uploads to Telegram
//! - Replicas periodically download and apply the latest snapshot

use crate::crypto::{decrypt, encrypt, EncryptedData, KEY_SIZE};
use crate::error::{Error, Result};
use crate::metadata::{Inode, MetadataStore};
use crate::telegram::TelegramBackend;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Replication role for a machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicationRole {
    /// Master node - can write and creates snapshots
    Master,
    /// Replica node - read-only, syncs from master
    Replica,
}

impl ReplicationRole {
    /// Check if this role can write
    pub fn can_write(&self) -> bool {
        matches!(self, ReplicationRole::Master)
    }

    /// Check if this role is a replica
    pub fn is_replica(&self) -> bool {
        matches!(self, ReplicationRole::Replica)
    }
}

/// Metadata snapshot for replication
///
/// This is a serializable snapshot of all inodes in the filesystem.
/// Chunks are content-addressed and immutable, so only metadata needs to be replicated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataSnapshot {
    /// Unique snapshot ID
    pub id: String,

    /// Machine ID of the master that created this snapshot
    pub master_id: Uuid,

    /// Namespace this snapshot belongs to
    pub namespace_id: String,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Snapshot version number (monotonically increasing)
    pub version: u64,

    /// All inodes in the filesystem (ino -> inode)
    pub inodes: HashMap<u64, Inode>,

    /// Next available inode number
    pub next_ino: u64,

    /// Optional description
    pub description: Option<String>,
}

impl MetadataSnapshot {
    /// Create a new metadata snapshot
    pub fn new(
        master_id: Uuid,
        namespace_id: String,
        version: u64,
        inodes: HashMap<u64, Inode>,
        next_ino: u64,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            master_id,
            namespace_id,
            created_at: Utc::now(),
            version,
            inodes,
            next_ino,
            description: None,
        }
    }

    /// Get the size of this snapshot in inodes
    pub fn inode_count(&self) -> usize {
        self.inodes.len()
    }

    /// Serialize the snapshot
    pub fn serialize(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(|e| Error::Serialization(e.to_string()))
    }

    /// Deserialize a snapshot
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        bincode::deserialize(data).map_err(|e| Error::Deserialization(e.to_string()))
    }

    /// Add description to snapshot
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }
}

/// Snapshot metadata stored in the metadata store
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    /// Snapshot ID
    pub snapshot_id: String,

    /// Version number
    pub version: u64,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Telegram message ID where snapshot is stored
    pub message_id: i32,

    /// Size in bytes
    pub size_bytes: u64,

    /// Number of inodes
    pub inode_count: usize,
}

/// Manages snapshot creation, upload, download, and application
pub struct SnapshotManager {
    /// Encryption key for snapshots
    key: [u8; KEY_SIZE],

    /// Telegram backend for upload/download
    telegram: Arc<TelegramBackend>,

    /// Metadata store
    metadata_store: Arc<MetadataStore>,

    /// Current machine ID
    machine_id: Uuid,

    /// Namespace ID
    namespace_id: String,

    /// Current version number
    current_version: Arc<RwLock<u64>>,

    /// Maximum snapshots to retain (TODO: implement retention policy)
    #[allow(dead_code)]
    max_snapshots: usize,
}

impl SnapshotManager {
    /// Create a new snapshot manager
    pub fn new(
        key: [u8; KEY_SIZE],
        telegram: Arc<TelegramBackend>,
        metadata_store: Arc<MetadataStore>,
        machine_id: Uuid,
        namespace_id: String,
        max_snapshots: usize,
    ) -> Self {
        Self {
            key,
            telegram,
            metadata_store,
            machine_id,
            namespace_id,
            current_version: Arc::new(RwLock::new(0)),
            max_snapshots,
        }
    }

    /// Create a snapshot of the current metadata state
    pub async fn create_snapshot(&self) -> Result<MetadataSnapshot> {
        info!("Creating metadata snapshot for namespace {}", self.namespace_id);

        // Collect all inodes from the metadata store
        let mut inodes = HashMap::new();
        let mut max_ino = 1u64;

        // Walk all inodes starting from root
        let mut to_visit = vec![1u64]; // Start with root
        let mut visited = std::collections::HashSet::new();

        while let Some(ino) = to_visit.pop() {
            if visited.contains(&ino) {
                continue;
            }
            visited.insert(ino);

            if let Some(inode) = self.metadata_store.get_inode(ino)? {
                max_ino = max_ino.max(ino);

                // If it's a directory, add children to visit list
                if inode.is_dir() {
                    for child_ino in &inode.children {
                        to_visit.push(*child_ino);
                    }
                }

                inodes.insert(ino, inode);
            }
        }

        let next_ino = max_ino + 1;

        // Increment version
        let mut version = self.current_version.write().await;
        *version += 1;
        let snapshot_version = *version;
        drop(version);

        let snapshot = MetadataSnapshot::new(
            self.machine_id,
            self.namespace_id.clone(),
            snapshot_version,
            inodes,
            next_ino,
        );

        info!(
            "Created snapshot {} with {} inodes (version {})",
            snapshot.id,
            snapshot.inode_count(),
            snapshot_version
        );

        Ok(snapshot)
    }

    /// Upload a snapshot to Telegram
    pub async fn upload_snapshot(&self, snapshot: &MetadataSnapshot) -> Result<i32> {
        info!("Uploading snapshot {} to Telegram", snapshot.id);

        // Serialize the snapshot
        let data = snapshot.serialize()?;
        debug!("Snapshot serialized to {} bytes", data.len());

        // Encrypt the data
        let encrypted = encrypt(&self.key, &data, &[])?;
        let encrypted_bytes = encrypted.to_bytes();
        debug!("Snapshot encrypted to {} bytes", encrypted_bytes.len());

        // Upload to Telegram with special metadata prefix
        let snapshot_filename = format!("tgfs_snapshot_{}_{}", self.namespace_id, snapshot.id);
        let message_id = self.telegram.upload_chunk(&snapshot_filename, &encrypted_bytes).await?;

        // Store snapshot metadata locally
        let metadata = SnapshotMetadata {
            snapshot_id: snapshot.id.clone(),
            version: snapshot.version,
            created_at: snapshot.created_at,
            message_id,
            size_bytes: encrypted_bytes.len() as u64,
            inode_count: snapshot.inode_count(),
        };

        let metadata_key = format!("snapshot_meta:{}", snapshot.id);
        let metadata_bytes = bincode::serialize(&metadata)?;
        self.metadata_store.save_metadata(&metadata_key, &metadata_bytes)?;

        info!(
            "Snapshot {} uploaded as message {} ({} bytes)",
            snapshot.id, message_id, encrypted_bytes.len()
        );

        // Clean up old snapshots
        self.cleanup_old_snapshots().await?;

        Ok(message_id)
    }

    /// Download the latest snapshot from Telegram
    pub async fn download_latest_snapshot(&self) -> Result<MetadataSnapshot> {
        info!("Downloading latest snapshot for namespace {}", self.namespace_id);

        // Find the latest snapshot metadata
        let latest_metadata = self.get_latest_snapshot_metadata()?;

        // Download from Telegram
        let encrypted_bytes = self.telegram.download_chunk(latest_metadata.message_id).await?;
        debug!("Downloaded {} bytes from Telegram", encrypted_bytes.len());

        // Decrypt
        let encrypted = EncryptedData::from_bytes(&encrypted_bytes)?;
        let decrypted = decrypt(&self.key, &encrypted, &[])?;
        debug!("Decrypted to {} bytes", decrypted.len());

        // Deserialize
        let snapshot = MetadataSnapshot::deserialize(&decrypted)?;

        info!(
            "Downloaded snapshot {} with {} inodes (version {})",
            snapshot.id,
            snapshot.inode_count(),
            snapshot.version
        );

        Ok(snapshot)
    }

    /// Apply a snapshot to the local metadata store (overwrite local state)
    pub async fn apply_snapshot(&self, snapshot: &MetadataSnapshot) -> Result<()> {
        info!(
            "Applying snapshot {} ({} inodes) to local metadata store",
            snapshot.id,
            snapshot.inode_count()
        );

        // This is a destructive operation - we're replacing all local metadata
        warn!(
            "Overwriting local metadata with snapshot version {}",
            snapshot.version
        );

        // Clear the cache first
        self.metadata_store.clear_cache();

        // Save all inodes from the snapshot
        for (ino, inode) in &snapshot.inodes {
            self.metadata_store.save_inode(inode)?;
            debug!("Applied inode {} ({})", ino, inode.name);
        }

        // Update version
        let mut version = self.current_version.write().await;
        *version = snapshot.version;
        drop(version);

        // Flush to disk
        self.metadata_store.flush()?;

        info!(
            "Successfully applied snapshot {} (version {})",
            snapshot.id, snapshot.version
        );

        Ok(())
    }

    /// Get metadata for the latest snapshot
    fn get_latest_snapshot_metadata(&self) -> Result<SnapshotMetadata> {
        // In a real implementation, this would scan the metadata store
        // for all snapshot metadata entries and return the one with the highest version

        // For now, return an error indicating no snapshot found
        // This will be implemented when we have a proper index
        Err(Error::SnapshotNotFound(
            "No snapshots found - snapshot indexing not yet implemented".to_string(),
        ))
    }

    /// Clean up old snapshots, keeping only the most recent N
    async fn cleanup_old_snapshots(&self) -> Result<()> {
        // TODO: Implement snapshot cleanup
        // 1. List all snapshot metadata
        // 2. Sort by version
        // 3. Keep the latest N, delete the rest
        debug!("Snapshot cleanup not yet implemented");
        Ok(())
    }

    /// Get the current version number
    pub async fn get_current_version(&self) -> u64 {
        *self.current_version.read().await
    }
}

/// Enforces read-only access on replica nodes
pub struct ReplicaEnforcer {
    /// The replication role
    role: ReplicationRole,

    /// Machine ID
    machine_id: Uuid,

    /// Namespace ID
    namespace_id: String,
}

impl ReplicaEnforcer {
    /// Create a new replica enforcer
    pub fn new(role: ReplicationRole, machine_id: Uuid, namespace_id: String) -> Self {
        Self {
            role,
            machine_id,
            namespace_id,
        }
    }

    /// Check if write operations are allowed
    ///
    /// Returns Ok(()) if writes are allowed, or an error with a clear message if not
    pub fn check_write_permission(&self) -> Result<()> {
        if self.role.can_write() {
            Ok(())
        } else {
            Err(Error::PermissionDenied)
        }
    }

    /// Check if read operations are allowed
    ///
    /// Always returns Ok for both master and replica
    pub fn check_read_permission(&self) -> Result<()> {
        Ok(())
    }

    /// Get the current role
    pub fn role(&self) -> ReplicationRole {
        self.role
    }

    /// Check if this is a replica
    pub fn is_replica(&self) -> bool {
        self.role.is_replica()
    }

    /// Get a human-readable error message for write denial
    pub fn write_denied_message(&self) -> String {
        format!(
            "Write operation denied: machine {} is a REPLICA for namespace '{}'. \
             Only the MASTER node can perform write operations. \
             This is a read-only replica.",
            self.machine_id, self.namespace_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replication_role() {
        assert!(ReplicationRole::Master.can_write());
        assert!(!ReplicationRole::Replica.can_write());
        assert!(ReplicationRole::Replica.is_replica());
        assert!(!ReplicationRole::Master.is_replica());
    }

    #[test]
    fn test_metadata_snapshot_creation() {
        let master_id = Uuid::new_v4();
        let namespace_id = "test".to_string();
        let mut inodes = HashMap::new();

        let root = Inode::root(1000, 1000, 0o755);
        inodes.insert(1, root);

        let snapshot = MetadataSnapshot::new(master_id, namespace_id, 1, inodes, 2);

        assert_eq!(snapshot.version, 1);
        assert_eq!(snapshot.inode_count(), 1);
        assert_eq!(snapshot.next_ino, 2);
    }

    #[test]
    fn test_metadata_snapshot_serialization() {
        let master_id = Uuid::new_v4();
        let namespace_id = "test".to_string();
        let mut inodes = HashMap::new();

        let root = Inode::root(1000, 1000, 0o755);
        inodes.insert(1, root);

        let snapshot = MetadataSnapshot::new(master_id, namespace_id.clone(), 1, inodes, 2);

        // Serialize and deserialize
        let serialized = snapshot.serialize().unwrap();
        let deserialized = MetadataSnapshot::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.id, snapshot.id);
        assert_eq!(deserialized.version, snapshot.version);
        assert_eq!(deserialized.namespace_id, namespace_id);
        assert_eq!(deserialized.inode_count(), 1);
    }

    #[test]
    fn test_replica_enforcer() {
        let machine_id = Uuid::new_v4();
        let namespace_id = "test".to_string();

        // Master can write
        let master_enforcer = ReplicaEnforcer::new(
            ReplicationRole::Master,
            machine_id,
            namespace_id.clone(),
        );
        assert!(master_enforcer.check_write_permission().is_ok());
        assert!(master_enforcer.check_read_permission().is_ok());

        // Replica cannot write
        let replica_enforcer = ReplicaEnforcer::new(
            ReplicationRole::Replica,
            machine_id,
            namespace_id,
        );
        assert!(replica_enforcer.check_write_permission().is_err());
        assert!(replica_enforcer.check_read_permission().is_ok());
        assert!(replica_enforcer.is_replica());
    }

    #[test]
    fn test_replica_enforcer_error_message() {
        let machine_id = Uuid::new_v4();
        let namespace_id = "production".to_string();

        let enforcer = ReplicaEnforcer::new(
            ReplicationRole::Replica,
            machine_id,
            namespace_id.clone(),
        );

        let message = enforcer.write_denied_message();
        assert!(message.contains("REPLICA"));
        assert!(message.contains("production"));
        assert!(message.contains("read-only"));
    }
}
