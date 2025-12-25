//! Version management for files
//!
//! Tracks file versions and provides access to historical versions.
//! Each version stores a snapshot of the chunk manifest.

use crate::chunk::ChunkManifest;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

/// A single file version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileVersion {
    /// Version number
    pub version: u64,
    /// Creation time
    pub created: SystemTime,
    /// File size at this version
    pub size: u64,
    /// Chunk manifest for this version
    pub manifest: ChunkManifest,
    /// Optional comment/description
    pub comment: Option<String>,
}

impl FileVersion {
    /// Create a new version
    pub fn new(version: u64, manifest: ChunkManifest, comment: Option<String>) -> Self {
        FileVersion {
            version,
            created: SystemTime::now(),
            size: manifest.total_size,
            manifest,
            comment,
        }
    }
}

/// Manages versions for files
pub struct VersionManager {
    /// Version history per inode
    versions: HashMap<u64, Vec<FileVersion>>,
    /// Maximum versions to keep per file (0 = unlimited)
    max_versions: usize,
}

impl VersionManager {
    /// Create a new version manager
    pub fn new(max_versions: usize) -> Self {
        VersionManager {
            versions: HashMap::new(),
            max_versions,
        }
    }

    /// Add a new version for a file
    pub fn add_version(&mut self, ino: u64, manifest: ChunkManifest, comment: Option<String>) -> u64 {
        let versions = self.versions.entry(ino).or_insert_with(Vec::new);

        // Determine next version number
        let next_version = versions.last().map(|v| v.version + 1).unwrap_or(1);

        // Create new version
        let version = FileVersion::new(next_version, manifest, comment);
        versions.push(version);

        // Prune old versions if needed
        if self.max_versions > 0 && versions.len() > self.max_versions {
            let to_remove = versions.len() - self.max_versions;
            versions.drain(..to_remove);
        }

        next_version
    }

    /// Get all versions for a file
    pub fn get_versions(&self, ino: u64) -> Option<&[FileVersion]> {
        self.versions.get(&ino).map(|v| v.as_slice())
    }

    /// Get a specific version
    pub fn get_version(&self, ino: u64, version: u64) -> Result<&FileVersion> {
        let versions = self.versions.get(&ino).ok_or(Error::InodeNotFound(ino))?;

        versions
            .iter()
            .find(|v| v.version == version)
            .ok_or(Error::VersionNotFound(version))
    }

    /// Get the latest version
    pub fn get_latest(&self, ino: u64) -> Option<&FileVersion> {
        self.versions.get(&ino).and_then(|v| v.last())
    }

    /// Delete all versions for a file
    pub fn delete_versions(&mut self, ino: u64) {
        self.versions.remove(&ino);
    }

    /// Get version count for a file
    pub fn version_count(&self, ino: u64) -> usize {
        self.versions.get(&ino).map(|v| v.len()).unwrap_or(0)
    }

    /// Get chunks that are only referenced by old versions
    /// (for garbage collection)
    pub fn get_orphaned_chunks(&self, ino: u64, current_manifest: &ChunkManifest) -> Vec<String> {
        let mut current_chunks: std::collections::HashSet<_> =
            current_manifest.chunks.iter().map(|c| c.id.clone()).collect();

        let mut all_chunks: std::collections::HashSet<String> = std::collections::HashSet::new();

        if let Some(versions) = self.versions.get(&ino) {
            for version in versions {
                for chunk in &version.manifest.chunks {
                    all_chunks.insert(chunk.id.clone());
                }
            }
        }

        // Chunks in old versions but not in current or any kept version
        all_chunks
            .difference(&current_chunks)
            .cloned()
            .collect()
    }

    /// Serialize version data for storage
    pub fn serialize(&self) -> Result<Vec<u8>> {
        bincode::serialize(&self.versions).map_err(|e| Error::Serialization(e.to_string()))
    }

    /// Deserialize version data
    pub fn deserialize(data: &[u8], max_versions: usize) -> Result<Self> {
        let versions: HashMap<u64, Vec<FileVersion>> =
            bincode::deserialize(data).map_err(|e| Error::Deserialization(e.to_string()))?;

        Ok(VersionManager {
            versions,
            max_versions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkRef;

    fn test_manifest(size: u64) -> ChunkManifest {
        ChunkManifest {
            version: 1,
            total_size: size,
            chunks: vec![ChunkRef {
                id: format!("chunk_{}", size),
                size: size,
                message_id: 1,
                offset: 0,
                original_size: size,
                compressed: false,
            }],
            file_hash: "test".to_string(),
        }
    }

    #[test]
    fn test_add_version() {
        let mut manager = VersionManager::new(10);

        let v1 = manager.add_version(1, test_manifest(100), None);
        let v2 = manager.add_version(1, test_manifest(200), Some("update".to_string()));

        assert_eq!(v1, 1);
        assert_eq!(v2, 2);
        assert_eq!(manager.version_count(1), 2);
    }

    #[test]
    fn test_version_limit() {
        let mut manager = VersionManager::new(2);

        manager.add_version(1, test_manifest(100), None);
        manager.add_version(1, test_manifest(200), None);
        manager.add_version(1, test_manifest(300), None);

        assert_eq!(manager.version_count(1), 2);

        // Should have kept versions 2 and 3
        let versions = manager.get_versions(1).unwrap();
        assert_eq!(versions[0].version, 2);
        assert_eq!(versions[1].version, 3);
    }

    #[test]
    fn test_get_version() {
        let mut manager = VersionManager::new(10);

        manager.add_version(1, test_manifest(100), None);
        manager.add_version(1, test_manifest(200), None);

        let v1 = manager.get_version(1, 1).unwrap();
        assert_eq!(v1.size, 100);

        let v2 = manager.get_version(1, 2).unwrap();
        assert_eq!(v2.size, 200);

        assert!(manager.get_version(1, 99).is_err());
    }

    #[test]
    fn test_get_latest() {
        let mut manager = VersionManager::new(10);

        manager.add_version(1, test_manifest(100), None);
        manager.add_version(1, test_manifest(200), None);

        let latest = manager.get_latest(1).unwrap();
        assert_eq!(latest.version, 2);
        assert_eq!(latest.size, 200);
    }

    #[test]
    fn test_delete_versions() {
        let mut manager = VersionManager::new(10);

        manager.add_version(1, test_manifest(100), None);
        manager.add_version(1, test_manifest(200), None);

        manager.delete_versions(1);

        assert_eq!(manager.version_count(1), 0);
        assert!(manager.get_latest(1).is_none());
    }
}
