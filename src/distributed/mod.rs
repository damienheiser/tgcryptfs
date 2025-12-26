//! Distributed infrastructure for TelegramFS
//!
//! This module provides support for multiple distribution modes:
//! - Standalone: Single machine, independent filesystem
//! - Namespace Isolation: Multiple independent filesystems on same Telegram account
//! - Master-Replica: One writer, multiple readers with sync
//! - CRDT Distributed: Full read/write from any node with automatic conflict resolution

pub mod crdt;
pub mod namespace;

pub use crdt::{
    Conflict, ConflictDetector, ConflictResolutionStrategy, ConflictResolver, ConflictType,
    CrdtOperation, CrdtSync, OperationLog, ResolutionResult, VectorClock,
};
pub use namespace::{
    AccessRule, AccessSubject, Namespace, NamespaceManager, NamespaceType, PermissionType,
    Permissions,
};
