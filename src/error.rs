//! Error types for tgcryptfs

use std::io;
use thiserror::Error;

/// Result type alias using our Error type
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for tgcryptfs
#[derive(Error, Debug)]
pub enum Error {
    // Crypto errors
    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: {0}")]
    Decryption(String),

    #[error("Key derivation error: {0}")]
    KeyDerivation(String),

    #[error("Invalid key length: expected {expected}, got {got}")]
    InvalidKeyLength { expected: usize, got: usize },

    // Telegram errors
    #[error("Telegram client error: {0}")]
    TelegramClient(String),

    #[error("Telegram authentication required")]
    TelegramAuthRequired,

    #[error("Telegram rate limited, retry after {seconds} seconds")]
    TelegramRateLimited { seconds: u32 },

    #[error("Telegram upload failed: {0}")]
    TelegramUpload(String),

    #[error("Telegram download failed: {0}")]
    TelegramDownload(String),

    #[error("Message not found: {0}")]
    MessageNotFound(i32),

    // Chunk errors
    #[error("Chunk not found: {0}")]
    ChunkNotFound(String),

    #[error("Chunk verification failed: expected {expected}, got {got}")]
    ChunkVerificationFailed { expected: String, got: String },

    #[error("Invalid chunk size: {0}")]
    InvalidChunkSize(usize),

    // Metadata errors
    #[error("Inode not found: {0}")]
    InodeNotFound(u64),

    #[error("Path not found: {0}")]
    PathNotFound(String),

    #[error("Not a directory: {0}")]
    NotADirectory(String),

    #[error("Not a file: {0}")]
    NotAFile(String),

    #[error("Directory not empty: {0}")]
    DirectoryNotEmpty(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Database error: {0}")]
    Database(#[from] sled::Error),

    // Filesystem errors
    #[error("Permission denied")]
    PermissionDenied,

    #[error("Invalid file handle: {0}")]
    InvalidFileHandle(u64),

    #[error("File too large: {size} bytes exceeds limit of {limit} bytes")]
    FileTooLarge { size: u64, limit: u64 },

    // Cache errors
    #[error("Cache miss: {0}")]
    CacheMiss(String),

    #[error("Cache full")]
    CacheFull,

    // Snapshot errors
    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("Snapshot already exists: {0}")]
    SnapshotAlreadyExists(String),

    // Version errors
    #[error("Version not found: {0}")]
    VersionNotFound(u64),

    // CRDT / Distributed errors
    #[error("Operation conflict: {0}")]
    OperationConflict(String),

    #[error("Operation not found: {0}")]
    OperationNotFound(String),

    #[error("Vector clock error: {0}")]
    VectorClock(String),

    #[error("Sync error: {0}")]
    Sync(String),

    #[error("Duplicate operation: {0}")]
    DuplicateOperation(String),

    // Erasure coding errors
    #[error("Erasure degraded: {available}/{required} accounts available")]
    ErasureDegraded { available: usize, required: usize },

    #[error("Erasure failed: only {available} accounts, need {required}")]
    ErasureFailed { available: usize, required: usize },

    #[error("Account {0} unavailable: {1}")]
    AccountUnavailable(u8, String),

    #[error("Stripe unrecoverable: only {available} of {required} blocks available")]
    StripeUnrecoverable { available: usize, required: usize },

    #[error("Erasure encoding failed: {0}")]
    ErasureEncode(String),

    #[error("Erasure decoding failed: {0}")]
    ErasureDecode(String),

    #[error("Invalid erasure configuration: {0}")]
    InvalidErasureConfig(String),

    #[error("Rebuild failed for account {account}: {reason}")]
    RebuildFailed { account: u8, reason: String },

    // Config errors
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    // IO errors
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    // Serialization errors
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    // General errors
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

impl Error {
    /// Convert to libc errno for FUSE
    pub fn to_errno(&self) -> libc::c_int {
        match self {
            Error::InodeNotFound(_) | Error::PathNotFound(_) | Error::ChunkNotFound(_) => {
                libc::ENOENT
            }
            Error::NotADirectory(_) => libc::ENOTDIR,
            Error::NotAFile(_) => libc::EISDIR,
            Error::DirectoryNotEmpty(_) => libc::ENOTEMPTY,
            Error::AlreadyExists(_) => libc::EEXIST,
            Error::PermissionDenied => libc::EACCES,
            Error::FileTooLarge { .. } => libc::EFBIG,
            Error::Io(e) => e.raw_os_error().unwrap_or(libc::EIO),
            Error::TelegramRateLimited { .. } => libc::EAGAIN,
            _ => libc::EIO,
        }
    }
}

impl From<bincode::Error> for Error {
    fn from(e: bincode::Error) -> Self {
        Error::Serialization(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serialization(e.to_string())
    }
}
