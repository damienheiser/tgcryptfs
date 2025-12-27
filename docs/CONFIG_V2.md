# tgcryptfs Configuration v2

## Overview

Configuration v2 introduces comprehensive support for distributed modes, multiple namespaces, and advanced synchronization features. This document describes the new configuration system.

## Key Features

### 1. Machine Identity
Every tgcryptfs instance has a unique identity:
- **Machine ID**: Auto-generated UUID or custom identifier
- **Machine Name**: Human-readable name for the machine

### 2. Distribution Modes
Three distribution modes are supported:

#### Standalone Mode
- Single machine, independent filesystem
- No synchronization
- Simplest setup

#### Master-Replica Mode
- One master (read/write) + multiple replicas (read-only)
- Periodic snapshot synchronization
- Ideal for backup and distribution scenarios

#### Distributed Mode
- Full CRDT-based multi-writer support
- All nodes can read and write
- Automatic conflict resolution
- Vector clock-based causality tracking

### 3. Namespaces
Logical isolation of filesystems:
- Multiple namespaces per machine
- Each namespace can have different types (standalone, master-replica, distributed)
- Per-namespace access control
- Independent mount points

### 4. Configuration Formats
- **YAML** (recommended): Human-readable, supports comments
- **JSON**: Machine-readable, strict syntax
- Format auto-detected from file extension

### 5. Environment Variable Substitution
Use `${VAR_NAME}` syntax in configs:
```yaml
telegram:
  api_id: ${TELEGRAM_APP_ID}
  api_hash: ${TELEGRAM_APP_HASH}
```

### 6. Validation
Comprehensive validation ensures:
- Required fields are present
- Mode-specific requirements are met
- Namespace configurations are consistent
- No invalid references

## Configuration Structure

```yaml
version: 2

# Machine identity
machine:
  id: "auto"  # or specific UUID/name
  name: "My Server"

# Telegram connection
telegram:
  api_id: ${TELEGRAM_APP_ID}
  api_hash: ${TELEGRAM_APP_HASH}
  session_file: "~/.tgcryptfs/session"
  max_concurrent_uploads: 3
  max_concurrent_downloads: 5
  retry_attempts: 3
  retry_base_delay_ms: 1000

# Encryption settings
encryption:
  argon2_memory_kib: 65536
  argon2_iterations: 3
  argon2_parallelism: 4
  salt: ""  # Auto-generated

# Distribution configuration
distribution:
  mode: distributed  # standalone | master-replica | distributed
  cluster_id: "home-cluster"

  # For master-replica mode
  master_replica:
    role: master  # master | replica
    master_id: "machine-id"
    sync_interval_secs: 60
    snapshot_retention: 10

  # For distributed mode
  distributed:
    sync_interval_ms: 1000
    conflict_resolution: last-write-wins  # last-write-wins | manual | merge
    operation_log_retention_hours: 168

# Namespace definitions
namespaces:
  - name: "private"
    type: standalone
    mount_point: "/mnt/private"

  - name: "shared"
    type: distributed
    cluster: "home-cluster"
    mount_point: "/mnt/shared"
    access:
      - machine: "machine-1"
        permissions: [read, write, delete, admin]
      - machine: "machine-2"
        permissions: [read, write]

# Cache settings
cache:
  max_size: 1073741824  # 1 GB
  cache_dir: "~/.tgcryptfs/cache"
  prefetch_enabled: true
  prefetch_count: 3
  eviction_policy: Lru

# Logging
logging:
  level: "info"
  file: "~/.tgcryptfs/tgcryptfs.log"
```

## CLI Commands

### Machine Management
```bash
# Initialize machine identity
tgcryptfs machine init
tgcryptfs machine init --name "my-server"

# Show machine info
tgcryptfs machine show
```

### Namespace Management
```bash
# Create namespaces
tgcryptfs namespace create <name> --type standalone
tgcryptfs namespace create <name> --type master-replica --master <master-id>
tgcryptfs namespace create <name> --type distributed --cluster <cluster-id>

# List namespaces
tgcryptfs namespace list
```

### Cluster Management
```bash
# Create new cluster
tgcryptfs cluster create <cluster-id>

# Join existing cluster
tgcryptfs cluster join <cluster-id> --role master
tgcryptfs cluster join <cluster-id> --role replica
tgcryptfs cluster join <cluster-id> --role node

# Show cluster status
tgcryptfs cluster status
```

### Sync Management
```bash
# Show sync status
tgcryptfs sync status

# Force immediate sync
tgcryptfs sync now
```

## Implementation Details

### File Structure
- **src/config.rs**: Configuration types and loading logic
  - `ConfigV2`: Main v2 configuration struct
  - `MachineConfig`: Machine identity
  - `DistributionConfig`: Distribution mode settings
  - `MasterReplicaConfig`: Master-replica specific config
  - `DistributedConfig`: CRDT distributed config
  - `NamespaceConfig`: Namespace definitions
  - `AccessRule`: Per-namespace permissions

### Validation Rules
1. **Telegram credentials required**: api_id and api_hash must be set
2. **Mode consistency**: cluster_id required for distributed modes
3. **Namespace validation**:
   - Master-replica namespaces require `master` field
   - Distributed namespaces require `cluster` field
4. **Reference validation**: All referenced machines/clusters must exist

### Environment Variable Substitution
- Regex pattern: `\$\{([A-Z_][A-Z0-9_]*)\}`
- Performs substitution before parsing
- Missing variables remain as-is (not an error)

### Backwards Compatibility
- Legacy `Config` (v1) struct still supported
- No `version` field = v1 config
- Can run both v1 and v2 configs
- Migration path provided in examples

## Examples

See the `examples/` directory for complete configuration examples:
- `config-v2-standalone.yaml`: Standalone mode
- `config-v2-master-replica.yaml`: Master-replica mode
- `config-v2-distributed.yaml`: Distributed CRDT mode

## Next Steps

The config v2 system is ready for use. Future work includes:
1. Implementing the actual distributed synchronization logic
2. CRDT operation handling
3. Conflict resolution strategies
4. Access control enforcement
5. Cluster member discovery and authentication

## Testing

```bash
# Verify config compiles
cargo check

# Run with example config
export TELEGRAM_APP_ID="your_id"
export TELEGRAM_APP_HASH="your_hash"
tgcryptfs machine init --name "test"
tgcryptfs cluster create "test-cluster"
tgcryptfs namespace create "test" --type distributed --cluster "test-cluster"
tgcryptfs cluster status
```
