# tgcryptfs Configuration Examples

This directory contains example configuration files for different tgcryptfs deployment modes.

## Configuration Files

### Standalone Mode
- **File**: `config-v2-standalone.yaml`
- **Use Case**: Single machine, independent filesystem
- **Features**: Simple setup, no synchronization

### Master-Replica Mode
- **File**: `config-v2-master-replica.yaml`
- **Use Case**: One writer (master), multiple readers (replicas)
- **Features**:
  - Master has full read/write access
  - Replicas have read-only access
  - Periodic snapshot synchronization
  - Suitable for backup/distribution scenarios

### Distributed Mode
- **File**: `config-v2-distributed.yaml`
- **Use Case**: Multiple machines with full read/write access
- **Features**:
  - CRDT-based conflict resolution
  - All nodes can read and write
  - Automatic synchronization
  - Configurable conflict resolution strategies

## Usage

### 1. Initialize Machine Identity

```bash
# Initialize with auto-generated UUID
tgcryptfs machine init

# Or specify a custom name
tgcryptfs machine init --name "my-server"
```

### 2. Create Configuration

Copy one of the example configs:

```bash
mkdir -p ~/.config/tgcryptfs
cp examples/config-v2-standalone.yaml ~/.config/tgcryptfs/config.yaml
```

### 3. Set Environment Variables

```bash
export TELEGRAM_APP_ID="your_api_id"
export TELEGRAM_APP_HASH="your_api_hash"
```

Or directly edit the config file to replace the `${TELEGRAM_APP_ID}` placeholders.

### 4. Create Namespaces

```bash
# Standalone namespace
tgcryptfs namespace create my-data --type standalone --mount-point /mnt/my-data

# Master-replica namespace (on master)
tgcryptfs namespace create shared --type master-replica --master "cloudyday-server" --mount-point /mnt/shared

# Distributed namespace
tgcryptfs namespace create collab --type distributed --cluster "home-cluster" --mount-point /mnt/collab
```

### 5. Cluster Operations

```bash
# Create a new cluster
tgcryptfs cluster create home-cluster

# Join existing cluster as master
tgcryptfs cluster join production --role master

# Join as replica
tgcryptfs cluster join production --role replica

# Join as distributed node
tgcryptfs cluster join home-cluster --role node

# Check cluster status
tgcryptfs cluster status
```

## Configuration Format

tgcryptfs supports both YAML (`.yaml`, `.yml`) and JSON (`.json`) formats. The format is auto-detected from the file extension.

### YAML (Recommended)
```yaml
version: 2
machine:
  id: "auto"
  name: "My Server"
# ...
```

### JSON
```json
{
  "version": 2,
  "machine": {
    "id": "auto",
    "name": "My Server"
  }
}
```

## Environment Variable Substitution

Use `${VAR_NAME}` syntax in config files:

```yaml
telegram:
  api_id: ${TELEGRAM_APP_ID}
  api_hash: ${TELEGRAM_APP_HASH}
  session_file: "${HOME}/.tgcryptfs/session"
```

## Configuration Validation

The config system validates:
- Required fields (API credentials, machine ID, etc.)
- Mode-specific requirements (cluster_id for distributed modes)
- Namespace consistency (master/cluster references)
- Field value constraints

Run validation explicitly:

```bash
tgcryptfs status
```

## Migration from v1

Legacy v1 configs (without `version` field) are still supported but deprecated. To migrate:

1. Add `version: 2` to your config
2. Add `machine` section
3. Add `distribution` section
4. Convert single mount to `namespaces` array
5. Add `logging` section (optional)

The old `Config` struct is maintained for backwards compatibility.
