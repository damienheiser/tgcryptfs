//! Distributed infrastructure for TelegramFS
//!
//! This module provides support for multiple distribution modes:
//! - Standalone: Single machine, independent filesystem
//! - Namespace Isolation: Multiple independent filesystems on same Telegram account
//! - Master-Replica: One writer, multiple readers with sync
//! - CRDT Distributed: Full read/write from any node with automatic conflict resolution

pub mod crdt;
pub mod identity;
pub mod namespace;
pub mod replication;
pub mod sync;
pub mod types;
pub mod vector_clock;

pub use crdt::{
    Conflict, ConflictDetector, ConflictResolutionStrategy, ConflictResolver, ConflictType,
    CrdtOperation, CrdtSync, OperationLog, ResolutionResult,
};
pub use identity::{IdentityStore, IdentityStoreError, MachineIdentity};
pub use namespace::{
    Namespace, NamespaceManager,
};
pub use replication::{
    MetadataSnapshot, ReplicaEnforcer, ReplicationRole, SnapshotManager,
};
pub use sync::{SyncDaemon, SyncStatus};
pub use types::{AccessRule, AccessSubject, NamespaceType, PermissionType, Permissions};
pub use vector_clock::{ClockOrdering, VectorClock};
