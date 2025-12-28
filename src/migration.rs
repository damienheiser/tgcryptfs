//! HKDF Migration Module for tgcryptfs
//!
//! This module handles migration of encrypted data from old HKDF purpose strings
//! (telegramfs-*) to new HKDF purpose strings (tgcryptfs-*).

use crate::crypto::{decrypt, encrypt, EncryptedData, KEY_SIZE, SALT_SIZE};
use crate::error::{Error, Result};
use ring::hkdf::{Salt, HKDF_SHA256};
use std::path::Path;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Old HKDF purpose strings (legacy)
const OLD_METADATA_PURPOSE: &[u8] = b"telegramfs-metadata-v1";
const OLD_CHUNK_PREFIX: &str = "telegramfs-chunk-v1:";
#[allow(dead_code)]
const OLD_MACHINE_PREFIX: &str = "telegramfs-machine-";

/// New HKDF purpose strings
const NEW_METADATA_PURPOSE: &[u8] = b"tgcryptfs-metadata-v1";
const NEW_CHUNK_PREFIX: &str = "tgcryptfs-chunk-v1:";
#[allow(dead_code)]
const NEW_MACHINE_PREFIX: &str = "tgcryptfs-machine-";

/// Derive a subkey using HKDF
fn derive_subkey(master_key: &[u8; KEY_SIZE], salt: &[u8], purpose: &[u8]) -> Result<[u8; KEY_SIZE]> {
    let hkdf_salt = Salt::new(HKDF_SHA256, salt);
    let prk = hkdf_salt.extract(master_key);

    let mut output = [0u8; KEY_SIZE];
    prk.expand(&[purpose], HkdfKeyType)
        .map_err(|_| Error::KeyDerivation("HKDF expansion failed".to_string()))?
        .fill(&mut output)
        .map_err(|_| Error::KeyDerivation("HKDF fill failed".to_string()))?;

    Ok(output)
}

struct HkdfKeyType;

impl ring::hkdf::KeyType for HkdfKeyType {
    fn len(&self) -> usize {
        KEY_SIZE
    }
}

/// Migration context for transitioning between HKDF schemes
pub struct HkdfMigration {
    master_key: Zeroizing<[u8; KEY_SIZE]>,
    salt: [u8; SALT_SIZE],

    // Keys derived with old purpose strings
    old_metadata_key: [u8; KEY_SIZE],

    // Keys derived with new purpose strings
    new_metadata_key: [u8; KEY_SIZE],
}

impl HkdfMigration {
    /// Create a new migration context
    pub fn new(master_key: &[u8; KEY_SIZE], salt: &[u8; SALT_SIZE]) -> Result<Self> {
        let old_metadata_key = derive_subkey(master_key, salt, OLD_METADATA_PURPOSE)?;
        let new_metadata_key = derive_subkey(master_key, salt, NEW_METADATA_PURPOSE)?;

        let mut key = [0u8; KEY_SIZE];
        key.copy_from_slice(master_key);

        Ok(Self {
            master_key: Zeroizing::new(key),
            salt: *salt,
            old_metadata_key,
            new_metadata_key,
        })
    }

    /// Get old chunk key for a chunk ID
    pub fn old_chunk_key(&self, chunk_id: &str) -> Result<[u8; KEY_SIZE]> {
        let purpose = format!("{}{}", OLD_CHUNK_PREFIX, chunk_id);
        derive_subkey(&self.master_key, &self.salt, purpose.as_bytes())
    }

    /// Get new chunk key for a chunk ID
    pub fn new_chunk_key(&self, chunk_id: &str) -> Result<[u8; KEY_SIZE]> {
        let purpose = format!("{}{}", NEW_CHUNK_PREFIX, chunk_id);
        derive_subkey(&self.master_key, &self.salt, purpose.as_bytes())
    }

    /// Get old metadata key
    pub fn old_metadata_key(&self) -> &[u8; KEY_SIZE] {
        &self.old_metadata_key
    }

    /// Get new metadata key
    pub fn new_metadata_key(&self) -> &[u8; KEY_SIZE] {
        &self.new_metadata_key
    }

    /// Re-encrypt data from old key to new key
    /// Input and output are raw bytes (nonce + ciphertext format)
    pub fn re_encrypt(&self, raw_ciphertext: &[u8], old_key: &[u8; KEY_SIZE], new_key: &[u8; KEY_SIZE]) -> Result<Vec<u8>> {
        // Parse the encrypted data
        let encrypted = EncryptedData::from_bytes(raw_ciphertext)?;

        // Decrypt with old key
        let plaintext = decrypt(old_key, &encrypted, &[])?;

        // Encrypt with new key
        let new_encrypted = encrypt(new_key, &plaintext, &[])?;

        // Return as raw bytes
        Ok(new_encrypted.to_bytes())
    }

    /// Re-encrypt metadata from old HKDF to new HKDF
    pub fn re_encrypt_metadata(&self, raw_ciphertext: &[u8]) -> Result<Vec<u8>> {
        self.re_encrypt(raw_ciphertext, &self.old_metadata_key, &self.new_metadata_key)
    }

    /// Re-encrypt a chunk from old HKDF to new HKDF
    pub fn re_encrypt_chunk(&self, raw_ciphertext: &[u8], chunk_id: &str) -> Result<Vec<u8>> {
        let old_key = self.old_chunk_key(chunk_id)?;
        let new_key = self.new_chunk_key(chunk_id)?;
        self.re_encrypt(raw_ciphertext, &old_key, &new_key)
    }
}

/// Migrate the local metadata database
pub fn migrate_metadata_db(
    db_path: &Path,
    migration: &HkdfMigration,
) -> Result<MigrationStats> {
    info!("Migrating metadata database at {:?}", db_path);

    let db = sled::open(db_path)?;

    let mut stats = MigrationStats::default();

    // Migrate all trees
    for tree_name in db.tree_names() {
        let tree_name_str = String::from_utf8_lossy(&tree_name);
        if tree_name_str == "__sled__default" {
            continue;
        }

        debug!("Migrating tree: {}", tree_name_str);

        let tree = db.open_tree(&tree_name)?;

        let mut updates = Vec::new();

        for item in tree.iter() {
            let (key, value) = item?;

            // Try to re-encrypt the value
            match migration.re_encrypt_metadata(&value) {
                Ok(new_value) => {
                    updates.push((key.to_vec(), new_value));
                    stats.entries_migrated += 1;
                }
                Err(e) => {
                    warn!("Failed to migrate entry in {}: {}", tree_name_str, e);
                    stats.entries_failed += 1;
                }
            }
        }

        // Apply updates
        for (key, value) in updates {
            tree.insert(&key, value)?;
        }

        tree.flush()?;
    }

    db.flush()?;

    info!(
        "Metadata migration complete: {} entries migrated, {} failed",
        stats.entries_migrated, stats.entries_failed
    );

    Ok(stats)
}

/// Statistics from a migration operation
#[derive(Debug, Default)]
pub struct MigrationStats {
    pub entries_migrated: usize,
    pub entries_failed: usize,
    pub chunks_migrated: usize,
    pub chunks_failed: usize,
    pub bytes_processed: u64,
}

impl MigrationStats {
    pub fn merge(&mut self, other: &MigrationStats) {
        self.entries_migrated += other.entries_migrated;
        self.entries_failed += other.entries_failed;
        self.chunks_migrated += other.chunks_migrated;
        self.chunks_failed += other.chunks_failed;
        self.bytes_processed += other.bytes_processed;
    }
}

/// Check if data is encrypted with old or new HKDF
pub fn detect_hkdf_version(
    raw_ciphertext: &[u8],
    old_key: &[u8; KEY_SIZE],
    new_key: &[u8; KEY_SIZE],
) -> HkdfVersion {
    // Parse encrypted data
    let encrypted = match EncryptedData::from_bytes(raw_ciphertext) {
        Ok(e) => e,
        Err(_) => return HkdfVersion::Unknown,
    };

    // Try decrypting with new key first (preferred)
    if decrypt(new_key, &encrypted, &[]).is_ok() {
        return HkdfVersion::New;
    }

    // Try old key
    if decrypt(old_key, &encrypted, &[]).is_ok() {
        return HkdfVersion::Old;
    }

    HkdfVersion::Unknown
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HkdfVersion {
    Old,    // telegramfs-*
    New,    // tgcryptfs-*
    Unknown,
}

impl std::fmt::Display for HkdfVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HkdfVersion::Old => write!(f, "telegramfs-* (legacy)"),
            HkdfVersion::New => write!(f, "tgcryptfs-* (current)"),
            HkdfVersion::Unknown => write!(f, "unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_master_key() -> [u8; KEY_SIZE] {
        let mut key = [0u8; KEY_SIZE];
        key[0] = 0x42;
        key
    }

    fn test_salt() -> [u8; SALT_SIZE] {
        let mut salt = [0u8; SALT_SIZE];
        salt[0] = 0x24;
        salt
    }

    #[test]
    fn test_migration_keys_different() {
        let migration = HkdfMigration::new(&test_master_key(), &test_salt())
            .expect("Failed to create migration");

        // Old and new keys should be different
        assert_ne!(migration.old_metadata_key(), migration.new_metadata_key());

        let old_chunk = migration.old_chunk_key("test-chunk").unwrap();
        let new_chunk = migration.new_chunk_key("test-chunk").unwrap();
        assert_ne!(old_chunk, new_chunk);
    }

    #[test]
    fn test_re_encryption() {
        let migration = HkdfMigration::new(&test_master_key(), &test_salt())
            .expect("Failed to create migration");

        let plaintext = b"Hello, tgcryptfs!";

        // Encrypt with old key
        let old_encrypted = encrypt(migration.old_metadata_key(), plaintext, &[])
            .expect("Failed to encrypt");
        let old_ciphertext = old_encrypted.to_bytes();

        // Re-encrypt
        let new_ciphertext = migration.re_encrypt_metadata(&old_ciphertext)
            .expect("Failed to re-encrypt");

        // Decrypt with new key
        let new_encrypted = EncryptedData::from_bytes(&new_ciphertext)
            .expect("Failed to parse");
        let decrypted = decrypt(migration.new_metadata_key(), &new_encrypted, &[])
            .expect("Failed to decrypt");

        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_detect_version() {
        let migration = HkdfMigration::new(&test_master_key(), &test_salt())
            .expect("Failed to create migration");

        let plaintext = b"test data";

        // Create ciphertext with old key
        let old_encrypted = encrypt(migration.old_metadata_key(), plaintext, &[])
            .expect("Failed to encrypt");
        let old_ciphertext = old_encrypted.to_bytes();

        let version = detect_hkdf_version(
            &old_ciphertext,
            migration.old_metadata_key(),
            migration.new_metadata_key(),
        );

        assert_eq!(version, HkdfVersion::Old);

        // Create ciphertext with new key
        let new_encrypted = encrypt(migration.new_metadata_key(), plaintext, &[])
            .expect("Failed to encrypt");
        let new_ciphertext = new_encrypted.to_bytes();

        let version = detect_hkdf_version(
            &new_ciphertext,
            migration.old_metadata_key(),
            migration.new_metadata_key(),
        );

        assert_eq!(version, HkdfVersion::New);
    }
}
