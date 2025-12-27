//! CRDT-based distributed write system for tgcryptfs
//!
//! This module implements Conflict-free Replicated Data Types (CRDTs) for
//! distributed filesystem operations. It enables multiple nodes to perform
//! concurrent writes with automatic conflict resolution.

use crate::distributed::VectorClock;
use crate::error::{Error, Result};
use crate::metadata::{FileType, InodeAttributes};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::SystemTime;
use uuid::Uuid;

/// CRDT operation types for filesystem operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CrdtOperation {
    /// Create a new file or directory
    Create {
        /// Unique operation ID
        op_id: Uuid,
        /// Machine that created this operation
        machine_id: Uuid,
        /// Vector clock at time of creation
        vector_clock: VectorClock,
        /// Wall clock timestamp
        timestamp: SystemTime,
        /// Parent directory path
        parent_path: String,
        /// Name of the new file/directory
        name: String,
        /// File type (file, directory, symlink)
        file_type: FileType,
        /// Initial file attributes
        initial_attrs: InodeAttributes,
        /// Symlink target (if file_type is Symlink)
        symlink_target: Option<String>,
    },

    /// Write data to a file
    Write {
        /// Unique operation ID
        op_id: Uuid,
        /// Machine that created this operation
        machine_id: Uuid,
        /// Vector clock at time of write
        vector_clock: VectorClock,
        /// Wall clock timestamp
        timestamp: SystemTime,
        /// Path to the file
        path: String,
        /// Offset in the file
        offset: u64,
        /// Hash of the data chunk (reference to chunk storage)
        data_hash: String,
        /// Length of the data
        length: u64,
    },

    /// Delete a file or directory (creates a tombstone)
    Delete {
        /// Unique operation ID
        op_id: Uuid,
        /// Machine that created this operation
        machine_id: Uuid,
        /// Vector clock at time of deletion
        vector_clock: VectorClock,
        /// Wall clock timestamp
        timestamp: SystemTime,
        /// Path to delete
        path: String,
        /// Tombstone timestamp for garbage collection
        tombstone_time: SystemTime,
    },

    /// Move/rename a file or directory
    Move {
        /// Unique operation ID
        op_id: Uuid,
        /// Machine that created this operation
        machine_id: Uuid,
        /// Vector clock at time of move
        vector_clock: VectorClock,
        /// Wall clock timestamp
        timestamp: SystemTime,
        /// Original path
        old_path: String,
        /// New path
        new_path: String,
    },

    /// Set file attributes
    SetAttr {
        /// Unique operation ID
        op_id: Uuid,
        /// Machine that created this operation
        machine_id: Uuid,
        /// Vector clock at time of attribute change
        vector_clock: VectorClock,
        /// Wall clock timestamp
        timestamp: SystemTime,
        /// Path to the file
        path: String,
        /// New attributes
        attrs: InodeAttributes,
    },
}

impl CrdtOperation {
    /// Get the operation ID
    pub fn op_id(&self) -> Uuid {
        match self {
            CrdtOperation::Create { op_id, .. }
            | CrdtOperation::Write { op_id, .. }
            | CrdtOperation::Delete { op_id, .. }
            | CrdtOperation::Move { op_id, .. }
            | CrdtOperation::SetAttr { op_id, .. } => *op_id,
        }
    }

    /// Get the machine ID that created this operation
    pub fn machine_id(&self) -> Uuid {
        match self {
            CrdtOperation::Create { machine_id, .. }
            | CrdtOperation::Write { machine_id, .. }
            | CrdtOperation::Delete { machine_id, .. }
            | CrdtOperation::Move { machine_id, .. }
            | CrdtOperation::SetAttr { machine_id, .. } => *machine_id,
        }
    }

    /// Get the vector clock
    pub fn vector_clock(&self) -> &VectorClock {
        match self {
            CrdtOperation::Create { vector_clock, .. }
            | CrdtOperation::Write { vector_clock, .. }
            | CrdtOperation::Delete { vector_clock, .. }
            | CrdtOperation::Move { vector_clock, .. }
            | CrdtOperation::SetAttr { vector_clock, .. } => vector_clock,
        }
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> SystemTime {
        match self {
            CrdtOperation::Create { timestamp, .. }
            | CrdtOperation::Write { timestamp, .. }
            | CrdtOperation::Delete { timestamp, .. }
            | CrdtOperation::Move { timestamp, .. }
            | CrdtOperation::SetAttr { timestamp, .. } => *timestamp,
        }
    }

    /// Get the path affected by this operation
    pub fn path(&self) -> &str {
        match self {
            CrdtOperation::Create { parent_path, .. } => {
                // Note: This is simplified; in practice you'd join paths properly
                parent_path
            }
            CrdtOperation::Write { path, .. }
            | CrdtOperation::Delete { path, .. }
            | CrdtOperation::SetAttr { path, .. } => path,
            CrdtOperation::Move { old_path, .. } => old_path,
        }
    }
}

/// Append-only log of CRDT operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationLog {
    /// All operations in chronological order
    operations: Vec<CrdtOperation>,
    /// Index: op_id -> position in operations vec
    op_index: HashMap<Uuid, usize>,
}

impl OperationLog {
    /// Create a new empty operation log
    pub fn new() -> Self {
        OperationLog {
            operations: Vec::new(),
            op_index: HashMap::new(),
        }
    }

    /// Append an operation to the log
    pub fn append(&mut self, op: CrdtOperation) -> Result<()> {
        let op_id = op.op_id();

        // Check for duplicate operations
        if self.op_index.contains_key(&op_id) {
            return Err(Error::Internal(format!(
                "Operation {} already exists in log",
                op_id
            )));
        }

        let index = self.operations.len();
        self.operations.push(op);
        self.op_index.insert(op_id, index);

        Ok(())
    }

    /// Get an operation by ID
    pub fn get(&self, op_id: &Uuid) -> Option<&CrdtOperation> {
        self.op_index.get(op_id).map(|&idx| &self.operations[idx])
    }

    /// Check if an operation exists
    pub fn contains(&self, op_id: &Uuid) -> bool {
        self.op_index.contains_key(op_id)
    }

    /// Get all operations
    pub fn operations(&self) -> &[CrdtOperation] {
        &self.operations
    }

    /// Get operations after a certain vector clock
    pub fn operations_after(&self, vc: &VectorClock) -> Vec<&CrdtOperation> {
        self.operations
            .iter()
            .filter(|op| op.vector_clock().happened_after(vc))
            .collect()
    }

    /// Get the number of operations
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Check if the log is empty
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

impl Default for OperationLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Detects conflicts between concurrent operations
#[derive(Debug)]
pub struct ConflictDetector {
    /// Current vector clock state
    current_clock: VectorClock,
}

impl ConflictDetector {
    /// Create a new conflict detector
    pub fn new(current_clock: VectorClock) -> Self {
        ConflictDetector { current_clock }
    }

    /// Detect if an operation conflicts with the current state
    pub fn detect_conflict(
        &self,
        op1: &CrdtOperation,
        op2: &CrdtOperation,
    ) -> Option<Conflict> {
        // Operations are concurrent if their vector clocks are concurrent
        if !op1.vector_clock().concurrent(op2.vector_clock()) {
            return None;
        }

        // Check if operations affect the same path
        let conflict_type = match (op1, op2) {
            // Two creates with same parent and name
            (
                CrdtOperation::Create { parent_path: p1, name: n1, .. },
                CrdtOperation::Create { parent_path: p2, name: n2, .. },
            ) if p1 == p2 && n1 == n2 => ConflictType::CreateCreate,

            // Write conflicts on same file
            (
                CrdtOperation::Write { path: path1, .. },
                CrdtOperation::Write { path: path2, .. },
            ) if path1 == path2 => ConflictType::WriteWrite,

            // Delete conflicts
            (
                CrdtOperation::Delete { path: path1, .. },
                CrdtOperation::Delete { path: path2, .. },
            ) if path1 == path2 => ConflictType::DeleteDelete,

            // Create vs Delete
            (
                CrdtOperation::Create { parent_path, name, .. },
                CrdtOperation::Delete { path, .. },
            ) if format!("{}/{}", parent_path, name) == *path => ConflictType::CreateDelete,

            // Delete vs Create
            (
                CrdtOperation::Delete { path, .. },
                CrdtOperation::Create { parent_path, name, .. },
            ) if *path == format!("{}/{}", parent_path, name) => ConflictType::DeleteCreate,

            // Move conflicts
            (
                CrdtOperation::Move { old_path: old1, .. },
                CrdtOperation::Move { old_path: old2, .. },
            ) if old1 == old2 => ConflictType::MoveMove,

            // SetAttr conflicts
            (
                CrdtOperation::SetAttr { path: path1, .. },
                CrdtOperation::SetAttr { path: path2, .. },
            ) if path1 == path2 => ConflictType::SetAttrSetAttr,

            _ => return None,
        };

        Some(Conflict {
            op1: op1.clone(),
            op2: op2.clone(),
            conflict_type,
        })
    }

    /// Update the current vector clock
    pub fn update_clock(&mut self, new_clock: VectorClock) {
        self.current_clock = new_clock;
    }
}

/// Represents a detected conflict between operations
#[derive(Debug, Clone)]
pub struct Conflict {
    pub op1: CrdtOperation,
    pub op2: CrdtOperation,
    pub conflict_type: ConflictType,
}

/// Types of conflicts that can occur
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictType {
    /// Two concurrent creates of the same file
    CreateCreate,
    /// Two concurrent writes to the same file
    WriteWrite,
    /// Two concurrent deletes of the same file
    DeleteDelete,
    /// Concurrent create and delete
    CreateDelete,
    /// Concurrent delete and create
    DeleteCreate,
    /// Two concurrent moves of the same file
    MoveMove,
    /// Two concurrent attribute changes
    SetAttrSetAttr,
}

/// Conflict resolution strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolutionStrategy {
    /// Last Write Wins (based on timestamp)
    LastWriteWins,
    /// Manual resolution required
    Manual,
    /// Attempt to merge changes automatically
    Merge,
}

/// Resolves conflicts between concurrent operations
#[derive(Debug)]
pub struct ConflictResolver {
    strategy: ConflictResolutionStrategy,
}

impl ConflictResolver {
    /// Create a new conflict resolver with the given strategy
    pub fn new(strategy: ConflictResolutionStrategy) -> Self {
        ConflictResolver { strategy }
    }

    /// Resolve a conflict and return the winning operation(s)
    pub fn resolve(&self, conflict: &Conflict) -> Result<ResolutionResult> {
        match self.strategy {
            ConflictResolutionStrategy::LastWriteWins => self.resolve_lww(conflict),
            ConflictResolutionStrategy::Manual => Ok(ResolutionResult::Manual(conflict.clone())),
            ConflictResolutionStrategy::Merge => self.resolve_merge(conflict),
        }
    }

    /// Resolve using Last Write Wins
    fn resolve_lww(&self, conflict: &Conflict) -> Result<ResolutionResult> {
        let ts1 = conflict.op1.timestamp();
        let ts2 = conflict.op2.timestamp();

        match ts1.cmp(&ts2) {
            std::cmp::Ordering::Greater => Ok(ResolutionResult::Winner(conflict.op1.clone())),
            std::cmp::Ordering::Less => Ok(ResolutionResult::Winner(conflict.op2.clone())),
            std::cmp::Ordering::Equal => {
                // Tie-breaker: use machine ID lexicographic order
                if conflict.op1.machine_id() < conflict.op2.machine_id() {
                    Ok(ResolutionResult::Winner(conflict.op1.clone()))
                } else {
                    Ok(ResolutionResult::Winner(conflict.op2.clone()))
                }
            }
        }
    }

    /// Resolve using merge strategy
    fn resolve_merge(&self, conflict: &Conflict) -> Result<ResolutionResult> {
        match conflict.conflict_type {
            ConflictType::WriteWrite => {
                // For concurrent writes, we keep both and let the application decide
                Ok(ResolutionResult::Merge(vec![
                    conflict.op1.clone(),
                    conflict.op2.clone(),
                ]))
            }
            ConflictType::SetAttrSetAttr => {
                // Merge attributes if possible
                Ok(ResolutionResult::Merge(vec![
                    conflict.op1.clone(),
                    conflict.op2.clone(),
                ]))
            }
            ConflictType::DeleteDelete => {
                // Both deletes win (idempotent)
                Ok(ResolutionResult::Winner(conflict.op1.clone()))
            }
            ConflictType::CreateCreate => {
                // Fall back to LWW for creates
                self.resolve_lww(conflict)
            }
            ConflictType::CreateDelete | ConflictType::DeleteCreate => {
                // Delete wins (conservative)
                let delete_op = if matches!(conflict.op1, CrdtOperation::Delete { .. }) {
                    conflict.op1.clone()
                } else {
                    conflict.op2.clone()
                };
                Ok(ResolutionResult::Winner(delete_op))
            }
            ConflictType::MoveMove => {
                // Fall back to LWW for moves
                self.resolve_lww(conflict)
            }
        }
    }
}

/// Result of conflict resolution
#[derive(Debug, Clone)]
pub enum ResolutionResult {
    /// Single winning operation
    Winner(CrdtOperation),
    /// Multiple operations to merge
    Merge(Vec<CrdtOperation>),
    /// Manual resolution required
    Manual(Conflict),
}

/// Main CRDT synchronization coordinator
pub struct CrdtSync {
    /// Current machine ID
    machine_id: Uuid,
    /// Current vector clock
    vector_clock: VectorClock,
    /// Local operation log
    operation_log: OperationLog,
    /// Set of operation IDs that have been applied
    applied_ops: HashSet<Uuid>,
    /// Pending operations to upload
    pending_ops: Vec<CrdtOperation>,
    /// Conflict resolver
    resolver: ConflictResolver,
}

impl CrdtSync {
    /// Create a new CRDT sync coordinator
    pub fn new(machine_id: Uuid, strategy: ConflictResolutionStrategy) -> Self {
        CrdtSync {
            machine_id,
            vector_clock: VectorClock::new(),
            operation_log: OperationLog::new(),
            applied_ops: HashSet::new(),
            pending_ops: Vec::new(),
            resolver: ConflictResolver::new(strategy),
        }
    }

    /// Record a new operation created by this machine
    pub fn record_operation(&mut self, op: CrdtOperation) -> Result<()> {
        // Update vector clock
        self.vector_clock.increment(self.machine_id);

        // Update operation's vector clock (assuming it's mutable or we reconstruct)
        // For simplicity, we'll add the operation as-is since it should already have the clock

        // Add to operation log
        self.operation_log.append(op.clone())?;

        // Mark as applied
        self.applied_ops.insert(op.op_id());

        // Add to pending uploads
        self.pending_ops.push(op);

        Ok(())
    }

    /// Get pending operations that need to be uploaded
    pub fn pending_operations(&self) -> &[CrdtOperation] {
        &self.pending_ops
    }

    /// Mark operations as uploaded
    pub fn mark_uploaded(&mut self, op_ids: &[Uuid]) {
        self.pending_ops.retain(|op| !op_ids.contains(&op.op_id()));
    }

    /// Download and merge remote operations
    pub fn merge_operations(&mut self, remote_ops: Vec<CrdtOperation>) -> Result<Vec<CrdtOperation>> {
        let mut new_ops = Vec::new();

        for remote_op in remote_ops {
            let op_id = remote_op.op_id();

            // Skip if already applied
            if self.applied_ops.contains(&op_id) {
                continue;
            }

            // Check for conflicts with existing operations
            let mut has_conflict = false;
            for local_op in self.operation_log.operations() {
                let detector = ConflictDetector::new(self.vector_clock.clone());
                if let Some(conflict) = detector.detect_conflict(&remote_op, local_op) {
                    has_conflict = true;

                    // Resolve the conflict
                    match self.resolver.resolve(&conflict)? {
                        ResolutionResult::Winner(winning_op) => {
                            if winning_op.op_id() == remote_op.op_id() {
                                new_ops.push(remote_op.clone());
                            }
                            // If local op wins, we don't apply remote op
                        }
                        ResolutionResult::Merge(ops) => {
                            // Apply all merged operations
                            for merged_op in ops {
                                if merged_op.op_id() == remote_op.op_id() {
                                    new_ops.push(merged_op);
                                }
                            }
                        }
                        ResolutionResult::Manual(_conflict) => {
                            return Err(Error::Internal(format!(
                                "Manual conflict resolution required for operation {}",
                                op_id
                            )));
                        }
                    }
                    break;
                }
            }

            // If no conflict, add the operation
            if !has_conflict {
                new_ops.push(remote_op.clone());
            }

            // Update state
            self.vector_clock.merge(remote_op.vector_clock());
            self.operation_log.append(remote_op)?;
            self.applied_ops.insert(op_id);
        }

        Ok(new_ops)
    }

    /// Get the current vector clock
    pub fn vector_clock(&self) -> &VectorClock {
        &self.vector_clock
    }

    /// Get the operation log
    pub fn operation_log(&self) -> &OperationLog {
        &self.operation_log
    }

    /// Get operations that occurred after a given vector clock
    pub fn operations_after(&self, vc: &VectorClock) -> Vec<&CrdtOperation> {
        self.operation_log.operations_after(vc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_log_append() {
        let mut log = OperationLog::new();
        let op_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();

        let op = CrdtOperation::Delete {
            op_id,
            machine_id,
            vector_clock: VectorClock::new(),
            timestamp: SystemTime::now(),
            path: "/test".to_string(),
            tombstone_time: SystemTime::now(),
        };

        assert!(log.is_empty());
        log.append(op).unwrap();
        assert_eq!(log.len(), 1);
        assert!(log.contains(&op_id));
    }

    #[test]
    fn test_operation_log_duplicate() {
        let mut log = OperationLog::new();
        let op_id = Uuid::new_v4();
        let machine_id = Uuid::new_v4();

        let op = CrdtOperation::Delete {
            op_id,
            machine_id,
            vector_clock: VectorClock::new(),
            timestamp: SystemTime::now(),
            path: "/test".to_string(),
            tombstone_time: SystemTime::now(),
        };

        log.append(op.clone()).unwrap();

        // Attempting to append the same operation again should fail
        assert!(log.append(op).is_err());
    }

    #[test]
    fn test_conflict_resolution_lww() {
        let resolver = ConflictResolver::new(ConflictResolutionStrategy::LastWriteWins);
        let machine1 = Uuid::new_v4();
        let machine2 = Uuid::new_v4();

        let ts1 = SystemTime::now();
        let ts2 = ts1 + std::time::Duration::from_secs(1);

        let op1 = CrdtOperation::Delete {
            op_id: Uuid::new_v4(),
            machine_id: machine1,
            vector_clock: VectorClock::new(),
            timestamp: ts1,
            path: "/test".to_string(),
            tombstone_time: ts1,
        };

        let op2 = CrdtOperation::Delete {
            op_id: Uuid::new_v4(),
            machine_id: machine2,
            vector_clock: VectorClock::new(),
            timestamp: ts2,
            path: "/test".to_string(),
            tombstone_time: ts2,
        };

        let conflict = Conflict {
            op1: op1.clone(),
            op2: op2.clone(),
            conflict_type: ConflictType::DeleteDelete,
        };

        let result = resolver.resolve(&conflict).unwrap();

        match result {
            ResolutionResult::Winner(op) => {
                assert_eq!(op.timestamp(), ts2); // Later timestamp wins
            }
            _ => panic!("Expected Winner result"),
        }
    }

    #[test]
    fn test_conflict_resolution_tie_breaker() {
        let resolver = ConflictResolver::new(ConflictResolutionStrategy::LastWriteWins);
        let machine1 = Uuid::new_v4();
        let machine2 = Uuid::new_v4();

        let ts = SystemTime::now();

        let op1 = CrdtOperation::Delete {
            op_id: Uuid::new_v4(),
            machine_id: machine1,
            vector_clock: VectorClock::new(),
            timestamp: ts,
            path: "/test".to_string(),
            tombstone_time: ts,
        };

        let op2 = CrdtOperation::Delete {
            op_id: Uuid::new_v4(),
            machine_id: machine2,
            vector_clock: VectorClock::new(),
            timestamp: ts,
            path: "/test".to_string(),
            tombstone_time: ts,
        };

        let conflict = Conflict {
            op1: op1.clone(),
            op2: op2.clone(),
            conflict_type: ConflictType::DeleteDelete,
        };

        let result = resolver.resolve(&conflict).unwrap();

        // Should resolve deterministically using machine ID
        match result {
            ResolutionResult::Winner(op) => {
                let expected_machine = if machine1 < machine2 { machine1 } else { machine2 };
                assert_eq!(op.machine_id(), expected_machine);
            }
            _ => panic!("Expected Winner result"),
        }
    }

    #[test]
    fn test_crdt_sync_record_operation() {
        let machine_id = Uuid::new_v4();
        let mut sync = CrdtSync::new(machine_id, ConflictResolutionStrategy::LastWriteWins);

        let op = CrdtOperation::Create {
            op_id: Uuid::new_v4(),
            machine_id,
            vector_clock: VectorClock::new(),
            timestamp: SystemTime::now(),
            parent_path: "/".to_string(),
            name: "test.txt".to_string(),
            file_type: FileType::RegularFile,
            initial_attrs: crate::metadata::InodeAttributes::new_file(1000, 1000, 0o644),
            symlink_target: None,
        };

        sync.record_operation(op.clone()).unwrap();

        assert_eq!(sync.pending_operations().len(), 1);
        assert_eq!(sync.operation_log().len(), 1);
    }
}
