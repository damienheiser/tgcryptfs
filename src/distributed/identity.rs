//! Machine identity management for distributed tgcryptfs

use crate::config::EncryptionConfig;
use crate::crypto::derive_key;
use crate::error::{Error, Result};
use ring::rand::SecureRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

/// Machine identity for distributed tgcryptfs
///
/// Each tgcryptfs instance has a unique identity that persists across restarts.
/// The identity includes a machine-specific encryption key derived from the master
/// password and the machine ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineIdentity {
    /// Unique machine ID (UUID v4)
    pub machine_id: Uuid,

    /// Human-readable machine name
    pub machine_name: String,

    /// Machine-specific encryption key (derived from master key + machine_id)
    /// This is used for encrypting local data that shouldn't be shared across machines
    #[serde(with = "serde_bytes")]
    pub machine_key: [u8; 32],

    /// Public key for cluster communication and authentication
    #[serde(with = "serde_bytes")]
    pub public_key: [u8; 32],

    /// Private key for signing (stored encrypted)
    #[serde(with = "serde_bytes_private_key")]
    private_key_seed: [u8; 32],

    /// First seen timestamp
    pub created_at: SystemTime,

    /// Last updated timestamp
    pub updated_at: SystemTime,
}

impl MachineIdentity {
    /// Generate a new machine identity
    ///
    /// # Arguments
    /// * `machine_name` - Human-readable name for this machine
    /// * `master_key` - The master encryption key (from password derivation)
    /// * `config` - Encryption configuration for key derivation
    ///
    /// # Returns
    /// A new MachineIdentity with generated UUID and derived keys
    pub fn generate(machine_name: String, master_key: &[u8; 32], config: &EncryptionConfig) -> Result<Self> {
        let machine_id = Uuid::new_v4();
        let now = SystemTime::now();

        // Derive machine-specific key from master key + machine ID
        let machine_key = Self::derive_machine_key(master_key, machine_id, config)?;

        // Generate Ed25519 key pair for signing
        let private_key_seed = {
            let mut seed = [0u8; 32];
            ring::rand::SystemRandom::new()
                .fill(&mut seed)
                .map_err(|_| Error::KeyDerivation("Failed to generate random seed".to_string()))?;
            seed
        };

        let key_pair = Ed25519KeyPair::from_seed_unchecked(&private_key_seed)
            .map_err(|_| Error::KeyDerivation("Failed to create Ed25519 key pair".to_string()))?;
        let public_key_bytes = key_pair.public_key().as_ref();
        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(public_key_bytes);

        Ok(Self {
            machine_id,
            machine_name,
            machine_key,
            public_key,
            private_key_seed,
            created_at: now,
            updated_at: now,
        })
    }

    /// Derive machine-specific encryption key from master key and machine ID
    ///
    /// This ensures each machine has its own encryption key even with the same master password
    fn derive_machine_key(master_key: &[u8; 32], machine_id: Uuid, config: &EncryptionConfig) -> Result<[u8; 32]> {
        let context = format!("tgcryptfs-machine-{}", machine_id);
        let derived = derive_key(master_key, Some(context.as_bytes()), config)?;
        let mut key = [0u8; 32];
        key.copy_from_slice(derived.key());
        Ok(key)
    }

    /// Get the Ed25519 key pair for signing
    pub fn key_pair(&self) -> Result<Ed25519KeyPair> {
        Ed25519KeyPair::from_seed_unchecked(&self.private_key_seed)
            .map_err(|_| Error::KeyDerivation("Failed to create key pair".to_string()))
    }

    /// Sign data with this machine's private key
    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>> {
        let key_pair = self.key_pair()?;
        Ok(key_pair.sign(data).as_ref().to_vec())
    }

    /// Verify a signature using this machine's public key
    pub fn verify(&self, data: &[u8], signature: &[u8]) -> bool {
        use ring::signature::{UnparsedPublicKey, ED25519};
        let public_key = UnparsedPublicKey::new(&ED25519, &self.public_key);
        public_key.verify(data, signature).is_ok()
    }

    /// Update the machine name
    pub fn set_name(&mut self, name: String) {
        self.machine_name = name;
        self.updated_at = SystemTime::now();
    }

    /// Serialize to bytes for storage
    pub fn to_bytes(&self) -> std::result::Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

// Custom serde module for private key seed
mod serde_bytes_private_key {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        bytes.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("invalid private key length"));
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(array)
    }
}

// Custom serde module for byte arrays
mod serde_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        bytes.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("invalid key length"));
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(array)
    }
}

/// Storage manager for machine identity
pub struct IdentityStore {
    db: sled::Tree,
}

impl IdentityStore {
    const IDENTITY_KEY: &'static [u8] = b"machine_identity";

    /// Create a new identity store
    pub fn new(db: sled::Db) -> std::result::Result<Self, sled::Error> {
        let tree = db.open_tree("machine")?;
        Ok(Self { db: tree })
    }

    /// Load the machine identity from storage
    pub fn load(&self) -> std::result::Result<Option<MachineIdentity>, IdentityStoreError> {
        match self.db.get(Self::IDENTITY_KEY)? {
            Some(bytes) => {
                let identity = MachineIdentity::from_bytes(&bytes)?;
                Ok(Some(identity))
            }
            None => Ok(None),
        }
    }

    /// Save the machine identity to storage
    pub fn save(&self, identity: &MachineIdentity) -> std::result::Result<(), IdentityStoreError> {
        let bytes = identity.to_bytes()?;
        self.db.insert(Self::IDENTITY_KEY, bytes.as_slice())?;
        self.db.flush()?;
        Ok(())
    }

    /// Get or create machine identity
    pub fn get_or_create(
        &self,
        machine_name: String,
        master_key: &[u8; 32],
        config: &EncryptionConfig,
    ) -> std::result::Result<MachineIdentity, IdentityStoreError> {
        if let Some(identity) = self.load()? {
            Ok(identity)
        } else {
            let identity = MachineIdentity::generate(machine_name, master_key, config)?;
            self.save(&identity)?;
            Ok(identity)
        }
    }

    /// Delete the machine identity (use with caution!)
    pub fn delete(&self) -> std::result::Result<(), sled::Error> {
        self.db.remove(Self::IDENTITY_KEY)?;
        self.db.flush()?;
        Ok(())
    }
}

/// Errors that can occur during identity storage operations
#[derive(Debug, thiserror::Error)]
pub enum IdentityStoreError {
    #[error("Database error: {0}")]
    Database(#[from] sled::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Error: {0}")]
    Error(#[from] Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_master_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        key[0] = 0x42; // Some test value
        key
    }

    fn test_config() -> EncryptionConfig {
        EncryptionConfig {
            argon2_memory_kib: 1024,
            argon2_iterations: 1,
            argon2_parallelism: 1,
            salt: Vec::new(),
        }
    }

    #[test]
    fn test_generate_identity() {
        let master_key = test_master_key();
        let config = test_config();
        let identity = MachineIdentity::generate("test-machine".to_string(), &master_key, &config)
            .expect("Failed to generate identity");

        assert_eq!(identity.machine_name, "test-machine");
        assert_ne!(identity.machine_id, Uuid::nil());
        assert_ne!(identity.machine_key, [0u8; 32]);
        assert_ne!(identity.public_key, [0u8; 32]);
    }

    #[test]
    fn test_machine_key_derivation() {
        let master_key = test_master_key();
        let config = test_config();
        let machine_id = Uuid::new_v4();

        let key1 = MachineIdentity::derive_machine_key(&master_key, machine_id, &config)
            .expect("Failed to derive key");
        let key2 = MachineIdentity::derive_machine_key(&master_key, machine_id, &config)
            .expect("Failed to derive key");

        // Same inputs should produce same key
        assert_eq!(key1, key2);

        // Different machine ID should produce different key
        let different_id = Uuid::new_v4();
        let key3 = MachineIdentity::derive_machine_key(&master_key, different_id, &config)
            .expect("Failed to derive key");
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_sign_and_verify() {
        let master_key = test_master_key();
        let config = test_config();
        let identity = MachineIdentity::generate("test-machine".to_string(), &master_key, &config)
            .expect("Failed to generate identity");

        let data = b"Hello, tgcryptfs!";
        let signature = identity.sign(data).expect("Failed to sign data");

        assert!(identity.verify(data, &signature));
        assert!(!identity.verify(b"Different data", &signature));
    }

    #[test]
    fn test_serialization() {
        let master_key = test_master_key();
        let config = test_config();
        let identity = MachineIdentity::generate("test-machine".to_string(), &master_key, &config)
            .expect("Failed to generate identity");

        let bytes = identity.to_bytes().expect("Failed to serialize");
        let deserialized =
            MachineIdentity::from_bytes(&bytes).expect("Failed to deserialize");

        assert_eq!(identity.machine_id, deserialized.machine_id);
        assert_eq!(identity.machine_name, deserialized.machine_name);
        assert_eq!(identity.machine_key, deserialized.machine_key);
        assert_eq!(identity.public_key, deserialized.public_key);
    }

    #[test]
    fn test_set_name() {
        let master_key = test_master_key();
        let config = test_config();
        let mut identity = MachineIdentity::generate("old-name".to_string(), &master_key, &config)
            .expect("Failed to generate identity");

        let original_updated = identity.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));

        identity.set_name("new-name".to_string());
        assert_eq!(identity.machine_name, "new-name");
        assert!(identity.updated_at > original_updated);
    }

    #[test]
    fn test_identity_store() {
        let sled_config = sled::Config::new().temporary(true);
        let db = sled_config.open().expect("Failed to open database");
        let store = IdentityStore::new(db).expect("Failed to create store");

        let master_key = test_master_key();
        let enc_config = test_config();

        // Initially empty
        assert!(store.load().expect("Failed to load").is_none());

        // Create and save
        let identity = MachineIdentity::generate("test-machine".to_string(), &master_key, &enc_config)
            .expect("Failed to generate identity");
        store.save(&identity).expect("Failed to save");

        // Load back
        let loaded = store
            .load()
            .expect("Failed to load")
            .expect("Identity not found");
        assert_eq!(identity.machine_id, loaded.machine_id);
        assert_eq!(identity.machine_name, loaded.machine_name);
    }

    #[test]
    fn test_get_or_create() {
        let sled_config = sled::Config::new().temporary(true);
        let db = sled_config.open().expect("Failed to open database");
        let store = IdentityStore::new(db).expect("Failed to create store");

        let master_key = test_master_key();
        let enc_config = test_config();

        // First call creates
        let identity1 = store
            .get_or_create("test-machine".to_string(), &master_key, &enc_config)
            .expect("Failed to get or create");

        // Second call retrieves existing
        let identity2 = store
            .get_or_create("different-name".to_string(), &master_key, &enc_config)
            .expect("Failed to get or create");

        // Should be the same identity (name not changed)
        assert_eq!(identity1.machine_id, identity2.machine_id);
        assert_eq!(identity1.machine_name, "test-machine");
    }
}
