# tgcryptfs Distributed Architecture

## Overview

tgcryptfs supports three distribution modes that can coexist:

1. **Standalone** - Single machine, independent filesystem
2. **Namespace Isolation** - Multiple independent filesystems on same Telegram account
3. **Master-Replica** - One writer, multiple readers with sync
4. **CRDT Distributed** - Full read/write from any node with automatic conflict resolution

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Telegram Saved Messages                           │
│                    (Immutable Encrypted Chunks)                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                  │
│  │ namespace:A │  │ namespace:B │  │ namespace:C │  (isolated)      │
│  └─────────────┘  └─────────────┘  └─────────────┘                  │
│  ┌─────────────────────────────────────────────────┐                │
│  │           shared:cluster-alpha                   │  (shared)     │
│  │  • Metadata snapshots                           │                │
│  │  • CRDT operation log                           │                │
│  │  • Chunk data                                   │                │
│  └─────────────────────────────────────────────────┘                │
└─────────────────────────────────────────────────────────────────────┘
                              ↑
         ┌────────────────────┼────────────────────┐
         ↓                    ↓                    ↓
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│   Machine A     │  │   Machine B     │  │   Machine C     │
│   (Master)      │  │   (Replica)     │  │   (CRDT Node)   │
│                 │  │                 │  │                 │
│ namespace:A     │  │ namespace:B     │  │ namespace:C     │
│ (standalone)    │  │ (standalone)    │  │ (standalone)    │
│                 │  │                 │  │                 │
│ shared:alpha    │  │ shared:alpha    │  │ shared:alpha    │
│ (write)     ────┼──┼→ (read-only) ←─┼──┼─ (full r/w)     │
└─────────────────┘  └─────────────────┘  └─────────────────┘
```

## Core Concepts

### 1. Machine Identity

Every tgcryptfs instance has a unique identity:

```rust
pub struct MachineIdentity {
    /// Unique machine ID (UUID v4)
    pub machine_id: Uuid,

    /// Human-readable name
    pub machine_name: String,

    /// Machine-specific encryption key (derived from master + machine_id)
    pub machine_key: [u8; 32],

    /// Public key for cluster communication
    pub public_key: [u8; 32],

    /// First seen timestamp
    pub created_at: SystemTime,
}
```

### 2. Namespaces

Namespaces provide logical isolation:

```rust
pub struct Namespace {
    /// Namespace identifier
    pub namespace_id: String,

    /// Namespace type
    pub namespace_type: NamespaceType,

    /// Encryption key for this namespace
    pub encryption_key: [u8; 32],

    /// Access control list
    pub acl: Vec<AccessRule>,

    /// Telegram message prefix for this namespace
    pub telegram_prefix: String,
}

pub enum NamespaceType {
    /// Private to this machine only
    Standalone,

    /// Shared with master-replica model
    MasterReplica {
        master_id: Uuid,
        replicas: Vec<Uuid>,
    },

    /// Shared with CRDT consensus
    Distributed {
        cluster_id: String,
        members: Vec<Uuid>,
    },
}
```

### 3. Vector Clocks

For causality tracking in distributed mode:

```rust
pub struct VectorClock {
    /// Machine ID -> logical timestamp
    pub clocks: HashMap<Uuid, u64>,
}

impl VectorClock {
    pub fn increment(&mut self, machine_id: Uuid) {
        *self.clocks.entry(machine_id).or_insert(0) += 1;
    }

    pub fn merge(&mut self, other: &VectorClock) {
        for (id, &time) in &other.clocks {
            let entry = self.clocks.entry(*id).or_insert(0);
            *entry = (*entry).max(time);
        }
    }

    pub fn happened_before(&self, other: &VectorClock) -> bool {
        // Returns true if self < other (causally)
    }

    pub fn concurrent(&self, other: &VectorClock) -> bool {
        // Returns true if neither happened before the other
    }
}
```

### 4. CRDT Operations

File operations as CRDTs:

```rust
pub enum CrdtOperation {
    /// Create file/directory
    Create {
        op_id: Uuid,
        machine_id: Uuid,
        vector_clock: VectorClock,
        parent_path: String,
        name: String,
        file_type: FileType,
        initial_attrs: InodeAttributes,
    },

    /// Write data (append-only log of writes)
    Write {
        op_id: Uuid,
        machine_id: Uuid,
        vector_clock: VectorClock,
        path: String,
        offset: u64,
        data_hash: String,  // Reference to chunk
        length: u64,
    },

    /// Delete (tombstone)
    Delete {
        op_id: Uuid,
        machine_id: Uuid,
        vector_clock: VectorClock,
        path: String,
        tombstone_time: SystemTime,
    },

    /// Rename/Move
    Move {
        op_id: Uuid,
        machine_id: Uuid,
        vector_clock: VectorClock,
        old_path: String,
        new_path: String,
    },

    /// Set attributes
    SetAttr {
        op_id: Uuid,
        machine_id: Uuid,
        vector_clock: VectorClock,
        path: String,
        attrs: InodeAttributes,
    },
}
```

## Distribution Modes

### Mode 1: Standalone (Default)

```yaml
distribution:
  mode: standalone
  machine_id: "auto"  # Generated on first run
  namespace: "default"
```

- Single machine, no sync
- Namespace prefix: `tgfs:{machine_id}:`
- Full read/write access

### Mode 2: Namespace Isolation

```yaml
distribution:
  mode: standalone
  machine_id: "machine-a"
  namespaces:
    - name: "private"
      type: standalone
    - name: "backups"
      type: standalone
```

- Multiple isolated filesystems
- Each namespace has separate metadata DB
- No cross-namespace visibility

### Mode 3: Master-Replica

```yaml
distribution:
  mode: master-replica
  machine_id: "cloudyday"
  cluster_id: "production"
  role: master  # or: replica

  master_replica:
    master_id: "cloudyday"  # Only master writes
    sync_interval_secs: 60
    snapshot_retention: 10

  namespaces:
    - name: "shared-storage"
      type: master-replica
      master: "cloudyday"
```

**Sync Protocol:**

1. Master writes operations to local DB
2. Master periodically snapshots metadata to Telegram
3. Replicas poll for new snapshots
4. Replicas apply snapshot (overwrite local state)
5. Replicas serve read-only access

```
Master                              Telegram                        Replica
  │                                    │                               │
  │ write(file)                        │                               │
  │────────────────────────────────────│                               │
  │ snapshot every 60s                 │                               │
  │───────────────────────────────────>│                               │
  │                                    │  poll every 60s               │
  │                                    │<──────────────────────────────│
  │                                    │  download snapshot            │
  │                                    │──────────────────────────────>│
  │                                    │                               │ apply
  │                                    │                               │ serve reads
```

### Mode 4: CRDT Distributed

```yaml
distribution:
  mode: distributed
  machine_id: "node-1"
  cluster_id: "home-cluster"

  distributed:
    sync_interval_ms: 1000
    conflict_resolution: last-write-wins  # or: manual, merge
    operation_log_retention_hours: 168  # 7 days

  namespaces:
    - name: "shared"
      type: distributed
      cluster: "home-cluster"
```

**CRDT Sync Protocol:**

1. Each write creates a `CrdtOperation`
2. Operations stored locally and uploaded to Telegram
3. Nodes periodically fetch operations from Telegram
4. Operations merged using vector clocks
5. Conflicts resolved by strategy (LWW, manual, merge)

```
Node A                          Telegram                          Node B
  │                                │                                 │
  │ write(file, v1)                │                                 │
  │ op: {vc:(A:1,B:0)}             │                                 │
  │───────────────────────────────>│                                 │
  │                                │      write(file, v2)            │
  │                                │      op: {vc:(A:0,B:1)}         │
  │                                │<────────────────────────────────│
  │         sync                   │                                 │
  │<───────────────────────────────│                                 │
  │         sync                   │                                 │
  │                                │────────────────────────────────>│
  │                                │                                 │
  │ detect: concurrent writes!     │      detect: concurrent writes! │
  │ resolve: LWW or merge          │      resolve: LWW or merge      │
```

## Data Storage Layout

### Telegram Message Format

```
Message Structure:
┌────────────────────────────────────────┐
│ Caption: tgfs:{namespace}:{type}:{id}  │
│ Document: encrypted_payload.bin        │
└────────────────────────────────────────┘

Types:
- chunk:{chunk_hash}     - File data chunk
- meta:{snapshot_id}     - Metadata snapshot
- op:{operation_id}      - CRDT operation
- manifest:{version}     - Cluster manifest
```

### Local Database Structure

```
sled_db/
├── machine/
│   └── identity           # MachineIdentity
├── namespaces/
│   ├── {ns}/inodes/       # Inode data
│   ├── {ns}/chunks/       # Chunk references
│   └── {ns}/tree/         # Directory tree
├── sync/
│   ├── vector_clock       # Current VC
│   ├── pending_ops/       # Ops to upload
│   └── applied_ops/       # Op IDs already applied
└── cluster/
    ├── members/           # Known cluster members
    └── snapshots/         # Snapshot metadata
```

## Access Control

```rust
pub struct AccessRule {
    /// Who this rule applies to
    pub subject: AccessSubject,

    /// What access is granted
    pub permissions: Permissions,

    /// Path pattern (glob)
    pub path_pattern: String,
}

pub enum AccessSubject {
    Machine(Uuid),
    MachineGroup(String),
    AnyAuthenticated,
    Public,
}

pub struct Permissions {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub admin: bool,  // Can modify ACLs
}
```

## Configuration

### Full Config Example

```yaml
# tgcryptfs Configuration
version: 2

# Machine identity
machine:
  id: "cloudyday-server"
  name: "Cloudyday Production Server"

# Telegram backend
telegram:
  api_id: ${TELEGRAM_APP_ID}
  api_hash: ${TELEGRAM_APP_HASH}
  session_file: "~/.tgcryptfs/session"

# Encryption
encryption:
  master_password: ${TGCRYPTFS_PASSWORD}
  kdf_iterations: 3
  kdf_memory_mb: 64

# Distribution mode
distribution:
  mode: distributed  # standalone | master-replica | distributed
  cluster_id: "home-cluster"

  # Master-Replica settings (if mode = master-replica)
  master_replica:
    role: master  # master | replica
    master_id: "cloudyday-server"
    sync_interval_secs: 60

  # CRDT Distributed settings (if mode = distributed)
  distributed:
    sync_interval_ms: 1000
    conflict_resolution: last-write-wins

# Namespaces
namespaces:
  # Private namespace (standalone)
  - name: "private"
    type: standalone
    mount_point: "/mnt/tgcryptfs/private"

  # Shared namespace (master-replica)
  - name: "backups"
    type: master-replica
    master: "cloudyday-server"
    mount_point: "/mnt/tgcryptfs/backups"

  # Fully distributed namespace
  - name: "shared"
    type: distributed
    cluster: "home-cluster"
    mount_point: "/mnt/tgcryptfs/shared"
    access:
      - machine: "cloudyday-server"
        permissions: [read, write, delete, admin]
      - machine: "pleasure-mac"
        permissions: [read, write]

# Cache settings
cache:
  enabled: true
  max_size_gb: 10
  path: "~/.tgcryptfs/cache"

# Logging
logging:
  level: info
  file: "~/.tgcryptfs/tgcryptfs.log"
```

## Implementation Phases

### Phase 1: Core Infrastructure
- [ ] Machine identity generation and storage
- [ ] Vector clock implementation
- [ ] Namespace isolation
- [ ] Config v2 parser

### Phase 2: Master-Replica
- [ ] Metadata snapshot creation
- [ ] Snapshot upload to Telegram
- [ ] Snapshot download and apply
- [ ] Read-only mode enforcement
- [ ] Sync status reporting

### Phase 3: CRDT Distributed
- [ ] CRDT operation types
- [ ] Operation serialization
- [ ] Operation log management
- [ ] Vector clock merging
- [ ] Conflict detection
- [ ] Conflict resolution strategies
- [ ] Automatic sync daemon

### Phase 4: Access Control
- [ ] ACL storage and evaluation
- [ ] Per-namespace permissions
- [ ] Machine authentication
- [ ] Audit logging

## API Additions

### CLI Commands

```bash
# Machine management
tgcryptfs machine init --name "my-machine"
tgcryptfs machine show
tgcryptfs machine list  # Show cluster members

# Namespace management
tgcryptfs namespace create <name> --type standalone|master-replica|distributed
tgcryptfs namespace list
tgcryptfs namespace mount <name> <path>

# Cluster management
tgcryptfs cluster create <cluster-id>
tgcryptfs cluster join <cluster-id> --role master|replica|node
tgcryptfs cluster status
tgcryptfs cluster members

# Sync management
tgcryptfs sync status
tgcryptfs sync now  # Force immediate sync
tgcryptfs sync history

# Access control
tgcryptfs acl set <namespace> --machine <id> --permissions read,write
tgcryptfs acl list <namespace>
```
