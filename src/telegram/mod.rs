//! Telegram backend module
//!
//! Handles all communication with Telegram's API including:
//! - Authentication and session management
//! - Uploading encrypted chunks as files
//! - Downloading chunks from Saved Messages
//! - Rate limiting and retry logic

mod client;
mod rate_limit;

pub use client::{TelegramBackend, TelegramMessage};
pub use rate_limit::RateLimiter;

/// Maximum file size for Telegram (2GB for premium, 1.5GB for regular)
pub const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024 * 1024; // 2GB

/// Prefix for chunk files uploaded to Telegram
pub const CHUNK_FILE_PREFIX: &str = "tgfs_chunk_";

/// Prefix for metadata files
pub const METADATA_FILE_PREFIX: &str = "tgfs_meta_";
