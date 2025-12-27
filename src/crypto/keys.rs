//! Key Management for tgcryptfs
//!
//! Implements a hierarchical key structure:
//! - Master Key: Derived from user password, protects metadata key and chunk keys
//! - Metadata Key: Encrypts filesystem metadata
//! - Chunk Keys: Per-chunk keys derived from master key + chunk ID

use crate::crypto::{derive_key, KEY_SIZE, SALT_SIZE};
use crate::config::EncryptionConfig;
use crate::error::{Error, Result};
use ring::hkdf::{self, Salt, HKDF_SHA256};
use std::sync::Arc;
use zeroize::{Zeroize, Zeroizing};

/// Master key derived from user password
pub struct MasterKey {
    /// The actual key material
    key: Zeroizing<[u8; KEY_SIZE]>,
    /// Salt used for derivation (needed for re-derivation)
    salt: [u8; SALT_SIZE],
}

impl MasterKey {
    /// Create a new master key from a password
    pub fn from_password(password: &[u8], config: &EncryptionConfig) -> Result<Self> {
        let salt = if config.salt.is_empty() {
            None
        } else {
            Some(config.salt.as_slice())
        };

        let derived = derive_key(password, salt, config)?;

        Ok(MasterKey {
            key: Zeroizing::new(*derived.key()),
            salt: *derived.salt(),
        })
    }

    /// Create from existing key material (for unlocking)
    pub fn from_existing(password: &[u8], salt: &[u8], config: &EncryptionConfig) -> Result<Self> {
        let derived = derive_key(password, Some(salt), config)?;

        Ok(MasterKey {
            key: Zeroizing::new(*derived.key()),
            salt: *derived.salt(),
        })
    }

    /// Get the raw key bytes
    pub fn key(&self) -> &[u8; KEY_SIZE] {
        &self.key
    }

    /// Get the salt
    pub fn salt(&self) -> &[u8; SALT_SIZE] {
        &self.salt
    }

    /// Derive a subkey for a specific purpose
    pub fn derive_subkey(&self, purpose: &[u8]) -> Result<[u8; KEY_SIZE]> {
        let salt = Salt::new(HKDF_SHA256, &self.salt);
        let prk = salt.extract(self.key.as_ref());

        let mut output = [0u8; KEY_SIZE];
        prk.expand(&[purpose], HkdfKeyType)
            .map_err(|_| Error::KeyDerivation("HKDF expansion failed".to_string()))?
            .fill(&mut output)
            .map_err(|_| Error::KeyDerivation("HKDF fill failed".to_string()))?;

        Ok(output)
    }

    /// Derive the metadata encryption key
    pub fn metadata_key(&self) -> Result<[u8; KEY_SIZE]> {
        self.derive_subkey(b"tgcryptfs-metadata-v1")
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        // Key is already wrapped in Zeroizing, but be explicit
    }
}

/// Per-chunk encryption key
#[derive(Clone)]
pub struct ChunkKey {
    key: Zeroizing<[u8; KEY_SIZE]>,
}

impl ChunkKey {
    /// Derive a chunk key from master key and chunk ID
    pub fn derive(master: &MasterKey, chunk_id: &str) -> Result<Self> {
        let purpose = format!("tgcryptfs-chunk-v1:{}", chunk_id);
        let key = master.derive_subkey(purpose.as_bytes())?;

        Ok(ChunkKey {
            key: Zeroizing::new(key),
        })
    }

    /// Get the raw key bytes
    pub fn key(&self) -> &[u8; KEY_SIZE] {
        &self.key
    }
}

/// HKDF key type for ring
struct HkdfKeyType;

impl hkdf::KeyType for HkdfKeyType {
    fn len(&self) -> usize {
        KEY_SIZE
    }
}

/// Key manager for the filesystem
pub struct KeyManager {
    master_key: Arc<MasterKey>,
    metadata_key: [u8; KEY_SIZE],
}

impl KeyManager {
    /// Create a new key manager from a master key
    pub fn new(master_key: MasterKey) -> Result<Self> {
        let metadata_key = master_key.metadata_key()?;

        Ok(KeyManager {
            master_key: Arc::new(master_key),
            metadata_key,
        })
    }

    /// Get the metadata encryption key
    pub fn metadata_key(&self) -> &[u8; KEY_SIZE] {
        &self.metadata_key
    }

    /// Get a chunk encryption key
    pub fn chunk_key(&self, chunk_id: &str) -> Result<ChunkKey> {
        ChunkKey::derive(&self.master_key, chunk_id)
    }

    /// Get the salt (needed for config persistence)
    pub fn salt(&self) -> &[u8; SALT_SIZE] {
        self.master_key.salt()
    }
}

impl Drop for KeyManager {
    fn drop(&mut self) {
        self.metadata_key.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EncryptionConfig {
        EncryptionConfig {
            argon2_memory_kib: 1024,
            argon2_iterations: 1,
            argon2_parallelism: 1,
            salt: Vec::new(),
        }
    }

    #[test]
    fn test_master_key_creation() {
        let config = test_config();
        let result = MasterKey::from_password(b"password", &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_master_key_deterministic() {
        let mut config = test_config();
        config.salt = vec![0u8; SALT_SIZE];

        let key1 = MasterKey::from_password(b"password", &config).unwrap();
        let key2 = MasterKey::from_password(b"password", &config).unwrap();

        assert_eq!(key1.key(), key2.key());
    }

    #[test]
    fn test_metadata_key_derivation() {
        let config = test_config();
        let master = MasterKey::from_password(b"password", &config).unwrap();

        let meta_key = master.metadata_key().unwrap();
        assert_eq!(meta_key.len(), KEY_SIZE);

        // Should be deterministic
        let meta_key2 = master.metadata_key().unwrap();
        assert_eq!(meta_key, meta_key2);
    }

    #[test]
    fn test_chunk_key_derivation() {
        let config = test_config();
        let master = MasterKey::from_password(b"password", &config).unwrap();

        let chunk1 = ChunkKey::derive(&master, "chunk-001").unwrap();
        let chunk2 = ChunkKey::derive(&master, "chunk-002").unwrap();

        // Different chunks get different keys
        assert_ne!(chunk1.key(), chunk2.key());

        // Same chunk ID gets same key
        let chunk1_again = ChunkKey::derive(&master, "chunk-001").unwrap();
        assert_eq!(chunk1.key(), chunk1_again.key());
    }

    #[test]
    fn test_key_manager() {
        let config = test_config();
        let master = MasterKey::from_password(b"password", &config).unwrap();
        let manager = KeyManager::new(master).unwrap();

        assert_eq!(manager.metadata_key().len(), KEY_SIZE);

        let chunk_key = manager.chunk_key("test-chunk").unwrap();
        assert_eq!(chunk_key.key().len(), KEY_SIZE);
    }
}
