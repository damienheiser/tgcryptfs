//! Cryptography module for tgcryptfs
//!
//! Provides AES-256-GCM encryption with Argon2id key derivation.
//! All data is encrypted before leaving the local system.

mod encryption;
mod kdf;
mod keys;

pub use encryption::{decrypt, encrypt, EncryptedData};
pub use kdf::{derive_key, DerivedKey};
pub use keys::{ChunkKey, KeyManager, MasterKey};

/// Size of AES-256 key in bytes
pub const KEY_SIZE: usize = 32;

/// Size of GCM nonce in bytes
pub const NONCE_SIZE: usize = 12;

/// Size of GCM authentication tag in bytes
pub const TAG_SIZE: usize = 16;

/// Size of salt for key derivation
pub const SALT_SIZE: usize = 32;
