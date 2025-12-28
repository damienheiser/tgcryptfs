//! Overlay filesystem module for tgcryptfs
//!
//! Provides an overlay filesystem where:
//! - Lower layer: User's local filesystem (read-only from overlay perspective)
//! - Upper layer: tgcryptfs encrypted cloud storage (stores modifications)
//! - Result: Merged view with copy-on-write semantics

mod config;
mod filesystem;
mod handle;
mod inode;
mod lower;
mod whiteout;

pub use config::{ConflictBehavior, OverlayConfig};
pub use filesystem::OverlayFs;
pub use handle::{OverlayFileHandle, OverlayHandleManager};
pub use inode::{InodeSource, OverlayAttributes, OverlayInode, OverlayInodeManager};
pub use lower::LowerLayer;
pub use whiteout::WhiteoutStore;
