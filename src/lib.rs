//! TelegramFS - Encrypted filesystem backed by Telegram Saved Messages
//!
//! This library provides a FUSE-based filesystem that stores all data
//! encrypted in Telegram's Saved Messages, with local caching for performance.

pub mod cache;
pub mod chunk;
pub mod config;
pub mod crypto;
pub mod error;
pub mod fs;
pub mod metadata;
pub mod snapshot;
pub mod telegram;

pub use config::Config;
pub use error::{Error, Result};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::config::Config;
    pub use crate::error::{Error, Result};
    pub use crate::metadata::Inode;
}
