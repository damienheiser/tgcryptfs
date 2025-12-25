//! FUSE filesystem implementation
//!
//! Implements the FUSE filesystem interface, translating
//! filesystem operations to our encrypted Telegram backend.

mod filesystem;
mod handle;

pub use filesystem::TelegramFs;
pub use handle::FileHandle;
