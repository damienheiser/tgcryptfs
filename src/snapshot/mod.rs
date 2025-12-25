//! Snapshot management module
//!
//! Provides point-in-time snapshots of the filesystem.
//! Snapshots are metadata-only (chunks are immutable).

mod snapshot;

pub use snapshot::{Snapshot, SnapshotManager};
