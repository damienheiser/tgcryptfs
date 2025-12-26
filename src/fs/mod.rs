//! FUSE filesystem implementation
//!
//! Implements the FUSE filesystem interface, translating
//! filesystem operations to our encrypted cloud backend.

mod filesystem;
mod handle;

pub use filesystem::TgCryptFs;
pub use handle::FileHandle;
