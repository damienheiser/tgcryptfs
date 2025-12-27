//! Distributed infrastructure for tgcryptfs
//!
//! This module provides support for multiple distribution modes:
//! - Standalone: Single machine, independent filesystem
//! - Namespace Isolation: Multiple independent filesystems on same Telegram account
//! - Master-Replica: One writer, multiple readers with sync
//! - CRDT Distributed: Full read/write from any node with automatic conflict resolution

// Core master-replica replication (implemented)
pub mod replication;
pub mod sync;

// Supporting modules
pub mod namespace;
pub mod types;

pub mod crdt;
pub mod vector_clock;

pub mod identity;

// Re-export master-replica types
pub use replication::{
    MetadataSnapshot, ReplicaEnforcer, ReplicationRole, SnapshotManager,
};
pub use sync::{SyncConfig, SyncDaemon, SyncStatus};

// Re-export supporting types
pub use namespace::{
    Namespace, NamespaceManager, PermissionType,
};
pub use types::{AccessRule, AccessSubject, NamespaceType, Permissions};

// Re-export CRDT types
pub use crdt::{
    Conflict, ConflictDetector, ConflictResolutionStrategy, ConflictResolver, ConflictType,
    CrdtOperation, CrdtSync, OperationLog, ResolutionResult,
};
pub use vector_clock::{ClockOrdering, VectorClock};

pub use identity::{IdentityStore, IdentityStoreError, MachineIdentity};
