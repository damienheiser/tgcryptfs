//! FUSE filesystem implementation
//!
//! Implements the FUSE filesystem interface, translating
//! filesystem operations to our encrypted cloud backend.

mod filesystem;
mod handle;
pub mod overlay;

pub use filesystem::TgCryptFs;
pub use handle::FileHandle;
pub use overlay::{OverlayConfig, OverlayFs};
