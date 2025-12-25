//! Key Derivation Functions using Argon2id
//!
//! Argon2id is the recommended algorithm for password hashing and key derivation.
//! It provides resistance against both side-channel and GPU-based attacks.

use crate::config::EncryptionConfig;
use crate::crypto::{KEY_SIZE, SALT_SIZE};
use crate::error::{Error, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use zeroize::Zeroizing;

/// Derived key with associated salt
#[derive(Clone)]
pub struct DerivedKey {
    /// The derived key material (zeroized on drop)
    key: Zeroizing<[u8; KEY_SIZE]>,
    /// Salt used for derivation
    salt: [u8; SALT_SIZE],
}

impl DerivedKey {
    /// Get the key bytes
    pub fn key(&self) -> &[u8; KEY_SIZE] {
        &self.key
    }

    /// Get the salt
    pub fn salt(&self) -> &[u8; SALT_SIZE] {
        &self.salt
    }
}

/// Derive a key from a password using Argon2id
///
/// # Arguments
/// * `password` - The password to derive from
/// * `salt` - Optional salt (generated if None)
/// * `config` - Encryption configuration with Argon2 parameters
///
/// # Returns
/// A DerivedKey containing the key material and salt
pub fn derive_key(
    password: &[u8],
    salt: Option<&[u8]>,
    config: &EncryptionConfig,
) -> Result<DerivedKey> {
    // Generate or use provided salt
    let mut salt_bytes = [0u8; SALT_SIZE];
    match salt {
        Some(s) if s.len() >= SALT_SIZE => {
            salt_bytes.copy_from_slice(&s[..SALT_SIZE]);
        }
        Some(s) => {
            return Err(Error::KeyDerivation(format!(
                "Salt too short: {} bytes, need {}",
                s.len(),
                SALT_SIZE
            )));
        }
        None => {
            rand::thread_rng().fill_bytes(&mut salt_bytes);
        }
    }

    // Configure Argon2id
    let params = Params::new(
        config.argon2_memory_kib,
        config.argon2_iterations,
        config.argon2_parallelism,
        Some(KEY_SIZE),
    )
    .map_err(|e| Error::KeyDerivation(format!("Invalid Argon2 parameters: {}", e)))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    // Derive the key
    let mut key_bytes = Zeroizing::new([0u8; KEY_SIZE]);
    argon2
        .hash_password_into(password, &salt_bytes, key_bytes.as_mut())
        .map_err(|e| Error::KeyDerivation(format!("Key derivation failed: {}", e)))?;

    Ok(DerivedKey {
        key: key_bytes,
        salt: salt_bytes,
    })
}

/// Generate a random salt
pub fn generate_salt() -> [u8; SALT_SIZE] {
    let mut salt = [0u8; SALT_SIZE];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EncryptionConfig {
        EncryptionConfig {
            argon2_memory_kib: 1024, // Low for testing
            argon2_iterations: 1,
            argon2_parallelism: 1,
            salt: Vec::new(),
        }
    }

    #[test]
    fn test_derive_key_generates_salt() {
        let config = test_config();
        let result = derive_key(b"password", None, &config);
        assert!(result.is_ok());

        let key = result.unwrap();
        assert_eq!(key.key().len(), KEY_SIZE);
        assert_eq!(key.salt().len(), SALT_SIZE);
    }

    #[test]
    fn test_derive_key_with_salt() {
        let config = test_config();
        let salt = generate_salt();

        let key1 = derive_key(b"password", Some(&salt), &config).unwrap();
        let key2 = derive_key(b"password", Some(&salt), &config).unwrap();

        assert_eq!(key1.key(), key2.key());
    }

    #[test]
    fn test_different_passwords_different_keys() {
        let config = test_config();
        let salt = generate_salt();

        let key1 = derive_key(b"password1", Some(&salt), &config).unwrap();
        let key2 = derive_key(b"password2", Some(&salt), &config).unwrap();

        assert_ne!(key1.key(), key2.key());
    }
}
