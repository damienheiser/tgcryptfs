//! AES-256-GCM Encryption Implementation
//!
//! All data is encrypted using AES-256-GCM which provides:
//! - Confidentiality: Data is encrypted
//! - Integrity: Any tampering is detected
//! - Authentication: Verifies the data came from the key holder

use crate::crypto::{KEY_SIZE, NONCE_SIZE, TAG_SIZE};
use crate::error::{Error, Result};
use rand::RngCore;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use serde::{Deserialize, Serialize};

/// Encrypted data container with nonce and authentication tag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    /// Nonce used for encryption (unique per encryption)
    #[serde(with = "serde_bytes")]
    pub nonce: Vec<u8>,
    /// Ciphertext with appended authentication tag
    #[serde(with = "serde_bytes")]
    pub ciphertext: Vec<u8>,
}

impl EncryptedData {
    /// Get the total size of encrypted data
    pub fn size(&self) -> usize {
        self.nonce.len() + self.ciphertext.len()
    }

    /// Serialize to bytes for storage
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(NONCE_SIZE + self.ciphertext.len());
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < NONCE_SIZE + TAG_SIZE {
            return Err(Error::Decryption("Data too short".to_string()));
        }

        Ok(EncryptedData {
            nonce: bytes[..NONCE_SIZE].to_vec(),
            ciphertext: bytes[NONCE_SIZE..].to_vec(),
        })
    }
}

/// Encrypt data using AES-256-GCM
///
/// # Arguments
/// * `key` - 256-bit encryption key
/// * `plaintext` - Data to encrypt
/// * `aad` - Additional authenticated data (optional, authenticated but not encrypted)
///
/// # Returns
/// EncryptedData containing nonce and ciphertext with auth tag
pub fn encrypt(key: &[u8; KEY_SIZE], plaintext: &[u8], aad: &[u8]) -> Result<EncryptedData> {
    // Create the key
    let unbound_key = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| Error::Encryption("Failed to create encryption key".to_string()))?;
    let sealing_key = LessSafeKey::new(unbound_key);

    // Generate random nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Prepare buffer: plaintext + space for tag
    let mut in_out = plaintext.to_vec();
    in_out.reserve(TAG_SIZE);

    // Encrypt in place
    sealing_key
        .seal_in_place_append_tag(nonce, Aad::from(aad), &mut in_out)
        .map_err(|_| Error::Encryption("Encryption failed".to_string()))?;

    Ok(EncryptedData {
        nonce: nonce_bytes.to_vec(),
        ciphertext: in_out,
    })
}

/// Decrypt data using AES-256-GCM
///
/// # Arguments
/// * `key` - 256-bit encryption key
/// * `encrypted` - Encrypted data container
/// * `aad` - Additional authenticated data (must match encryption)
///
/// # Returns
/// Decrypted plaintext
pub fn decrypt(key: &[u8; KEY_SIZE], encrypted: &EncryptedData, aad: &[u8]) -> Result<Vec<u8>> {
    if encrypted.nonce.len() != NONCE_SIZE {
        return Err(Error::Decryption(format!(
            "Invalid nonce length: {}",
            encrypted.nonce.len()
        )));
    }

    if encrypted.ciphertext.len() < TAG_SIZE {
        return Err(Error::Decryption("Ciphertext too short".to_string()));
    }

    // Create the key
    let unbound_key = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| Error::Decryption("Failed to create decryption key".to_string()))?;
    let opening_key = LessSafeKey::new(unbound_key);

    // Create nonce
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    nonce_bytes.copy_from_slice(&encrypted.nonce);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Decrypt in place
    let mut in_out = encrypted.ciphertext.clone();
    let plaintext = opening_key
        .open_in_place(nonce, Aad::from(aad), &mut in_out)
        .map_err(|_| Error::Decryption("Decryption failed - data corrupted or wrong key".to_string()))?;

    Ok(plaintext.to_vec())
}

/// Encrypt with empty AAD (convenience function)
#[allow(dead_code)]
pub fn encrypt_simple(key: &[u8; KEY_SIZE], plaintext: &[u8]) -> Result<EncryptedData> {
    encrypt(key, plaintext, &[])
}

/// Decrypt with empty AAD (convenience function)
#[allow(dead_code)]
pub fn decrypt_simple(key: &[u8; KEY_SIZE], encrypted: &EncryptedData) -> Result<Vec<u8>> {
    decrypt(key, encrypted, &[])
}

mod serde_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; KEY_SIZE] {
        let mut key = [0u8; KEY_SIZE];
        rand::thread_rng().fill_bytes(&mut key);
        key
    }

    #[test]
    fn test_encrypt_decrypt() {
        let key = test_key();
        let plaintext = b"Hello, tgcryptfs!";

        let encrypted = encrypt_simple(&key, plaintext).unwrap();
        let decrypted = decrypt_simple(&key, &encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_with_aad() {
        let key = test_key();
        let plaintext = b"Secret data";
        let aad = b"file:1234";

        let encrypted = encrypt(&key, plaintext, aad).unwrap();
        let decrypted = decrypt(&key, &encrypted, aad).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_aad_fails() {
        let key = test_key();
        let plaintext = b"Secret data";
        let aad = b"file:1234";
        let wrong_aad = b"file:5678";

        let encrypted = encrypt(&key, plaintext, aad).unwrap();
        let result = decrypt(&key, &encrypted, wrong_aad);

        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = test_key();
        let key2 = test_key();
        let plaintext = b"Secret data";

        let encrypted = encrypt_simple(&key1, plaintext).unwrap();
        let result = decrypt_simple(&key2, &encrypted);

        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let key = test_key();
        let plaintext = b"Secret data";

        let mut encrypted = encrypt_simple(&key, plaintext).unwrap();
        // Tamper with ciphertext
        encrypted.ciphertext[0] ^= 0xFF;

        let result = decrypt_simple(&key, &encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_plaintext() {
        let key = test_key();
        let plaintext = b"";

        let encrypted = encrypt_simple(&key, plaintext).unwrap();
        let decrypted = decrypt_simple(&key, &encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_large_plaintext() {
        let key = test_key();
        let plaintext = vec![0x42u8; 1024 * 1024]; // 1MB

        let encrypted = encrypt_simple(&key, &plaintext).unwrap();
        let decrypted = decrypt_simple(&key, &encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_serialization() {
        let key = test_key();
        let plaintext = b"Test serialization";

        let encrypted = encrypt_simple(&key, plaintext).unwrap();
        let bytes = encrypted.to_bytes();
        let restored = EncryptedData::from_bytes(&bytes).unwrap();

        let decrypted = decrypt_simple(&key, &restored).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
