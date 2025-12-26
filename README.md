# TelegramFS

An encrypted FUSE filesystem that stores all data in Telegram's Saved Messages, providing unlimited cloud storage with end-to-end encryption and distributed multi-machine synchronization.

## Overview

TelegramFS mounts as a standard filesystem on your computer, but all files are:
1. **Encrypted locally** using AES-256-GCM with keys derived from your password
2. **Chunked** into manageable pieces (default 50MB)
3. **Compressed** using LZ4 when beneficial
4. **Deduplicated** using content-addressable storage (BLAKE3 hashes)
5. **Uploaded** to your Telegram Saved Messages as documents
6. **Synchronized** across multiple machines with conflict resolution

Your data remains encrypted end-to-end — Telegram only sees encrypted blobs.

## Features

### Core Features
- **End-to-End Encryption**: AES-256-GCM encryption with Argon2id key derivation
- **FUSE Filesystem**: Mount and use like any normal directory
- **Content Deduplication**: Identical data stored only once
- **LZ4 Compression**: Fast compression for compressible data
- **Local Caching**: LRU cache for fast repeated access
- **File Versioning**: Keep history of file changes
- **Snapshots**: Point-in-time filesystem snapshots
- **Cross-Platform**: Works on Linux and macOS

### Distributed Features (New!)
- **Machine Identity**: Each instance has a unique cryptographic identity (Ed25519)
- **Namespace Isolation**: Multiple independent filesystems on the same Telegram account
- **Master-Replica Mode**: One writer, multiple read-only replicas with automatic sync
- **CRDT Distributed Mode**: Full read/write from any node with automatic conflict resolution
- **Vector Clocks**: Causality tracking for distributed operations
- **Access Control**: Per-namespace ACLs with machine-level permissions

## Distribution Modes

### Standalone (Default)
Single machine, independent filesystem. Simplest setup.

```yaml
distribution:
  mode: standalone
```

### Master-Replica
One master (read/write) with multiple replicas (read-only). Ideal for backup and distribution.

```yaml
distribution:
  mode: master-replica
  cluster_id: "my-cluster"
  master_replica:
    role: master  # or: replica
    master_id: "main-server"
    sync_interval_secs: 60
```

### Distributed (CRDT)
Full multi-writer support with automatic conflict resolution using CRDTs.

```yaml
distribution:
  mode: distributed
  cluster_id: "home-cluster"
  distributed:
    sync_interval_ms: 1000
    conflict_resolution: last-write-wins  # or: manual, merge
```

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

## Quick Start

### Prerequisites

- Rust 1.70+
- FUSE (libfuse3 on Linux, macFUSE on macOS)
- Telegram API credentials from [my.telegram.org](https://my.telegram.org)

### Installation

```bash
# Clone the repository
git clone https://github.com/damienheiser/telegramfs.git
cd telegramfs

# Build
cargo build --release

# Install (optional)
cargo install --path .
```

### Initial Setup

```bash
# Initialize with your Telegram API credentials
telegramfs init --api-id YOUR_API_ID --api-hash YOUR_API_HASH

# Authenticate with Telegram
telegramfs auth --phone +1234567890
# Enter the code sent to your Telegram

# Mount the filesystem
telegramfs mount /mnt/telegram

# Use it like a normal filesystem!
cp ~/documents/* /mnt/telegram/
ls /mnt/telegram/

# Unmount when done
telegramfs unmount /mnt/telegram
```

## Commands

| Command | Description |
|---------|-------------|
| `telegramfs init` | Initialize configuration with Telegram API credentials |
| `telegramfs auth` | Authenticate with Telegram |
| `telegramfs mount <path>` | Mount the filesystem |
| `telegramfs unmount <path>` | Unmount the filesystem |
| `telegramfs status` | Show filesystem and connection status |
| `telegramfs snapshot <name>` | Create a named snapshot |
| `telegramfs snapshots` | List all snapshots |
| `telegramfs restore <name>` | Restore from a snapshot |
| `telegramfs cache` | Show cache statistics |
| `telegramfs cache --clear` | Clear the local cache |
| `telegramfs sync` | Sync local state with Telegram |

## Configuration

### Config v2 (YAML)

Configuration is stored in `~/.config/telegramfs/config.yaml`:

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
  session_file: "~/.telegramfs/session"
  max_concurrent_uploads: 3
  max_concurrent_downloads: 5

# Encryption settings
encryption:
  argon2_memory_kib: 65536
  argon2_iterations: 3
  argon2_parallelism: 4

# Distribution configuration
distribution:
  mode: standalone  # standalone | master-replica | distributed
  cluster_id: "home-cluster"

  # For master-replica mode
  master_replica:
    role: master
    master_id: "main-server"
    sync_interval_secs: 60
    snapshot_retention: 10

  # For distributed mode
  distributed:
    sync_interval_ms: 1000
    conflict_resolution: last-write-wins

# Namespaces (multiple isolated filesystems)
namespaces:
  - name: "private"
    type: standalone
    mount_point: "/mnt/telegramfs/private"

  - name: "shared"
    type: distributed
    cluster: "home-cluster"
    mount_point: "/mnt/telegramfs/shared"
    access:
      - machine: "server-1"
        permissions: [read, write, delete, admin]
      - machine: "laptop"
        permissions: [read, write]

# Cache settings
cache:
  enabled: true
  max_size_gb: 10
  path: "~/.telegramfs/cache"
```

### Legacy Config v1 (JSON)

```json
{
  "telegram": {
    "api_id": 12345678,
    "api_hash": "your_api_hash",
    "max_concurrent_uploads": 3,
    "max_concurrent_downloads": 5
  },
  "encryption": {
    "argon2_memory_kib": 65536,
    "argon2_iterations": 3,
    "argon2_parallelism": 4
  },
  "cache": {
    "max_size": 1073741824,
    "prefetch_enabled": true,
    "prefetch_count": 3
  },
  "chunk": {
    "chunk_size": 52428800,
    "compression_enabled": true,
    "dedup_enabled": true
  },
  "versioning": {
    "enabled": true,
    "max_versions": 10
  }
}
```

## Security Model

### Key Hierarchy

```
Password
    │
    └─► Argon2id ─► Master Key
                        │
                        ├─► HKDF ─► Metadata Key (encrypts filesystem metadata)
                        │
                        ├─► HKDF ─► Chunk Keys (per-chunk encryption keys)
                        │
                        └─► HKDF ─► Machine Key (per-machine derived key)
```

### Machine Identity

Each TelegramFS instance has a unique cryptographic identity:
- **Machine ID**: UUID v4 for unique identification
- **Ed25519 Key Pair**: For signing and authentication
- **Machine Key**: Derived from master key + machine ID

### Encryption Details

- **Key Derivation**: Argon2id with configurable memory/time/parallelism
- **Encryption**: AES-256-GCM (authenticated encryption)
- **Chunk Hashing**: BLAKE3 for content-addressing and deduplication
- **Nonce Generation**: Cryptographically random 12-byte nonces
- **Signing**: Ed25519 signatures for distributed operations

### What Telegram Sees

Telegram only stores encrypted blobs with random-looking filenames. It cannot:
- Read your file contents
- See file names or directory structure
- Know how many files you have (only chunk count)
- Correlate chunks to files

## How It Works

### Writing a File

1. File data is split into fixed-size chunks (default 50MB)
2. Each chunk is hashed with BLAKE3 for content-addressing
3. Chunks are compressed with LZ4 if compression helps
4. Each chunk is encrypted with a derived per-chunk key
5. Encrypted chunks are uploaded to Telegram Saved Messages
6. Metadata (inodes, directory structure) is encrypted and stored locally
7. In distributed mode, CRDT operations are broadcast to other nodes

### Reading a File

1. File metadata is looked up from the encrypted local database
2. Required chunks are identified from the file's manifest
3. Chunks are fetched from local cache or downloaded from Telegram
4. Chunks are decrypted and decompressed
5. Data is assembled and returned to the application

### Distributed Sync

1. Each write creates a CRDT operation with vector clock timestamp
2. Operations are stored locally and uploaded to Telegram
3. Other nodes periodically fetch new operations
4. Vector clocks determine causality and detect conflicts
5. Conflicts are resolved using configured strategy (LWW, manual, merge)

### Deduplication

Identical content produces identical chunk hashes, so:
- Copying a file doesn't re-upload data
- Modified files only upload changed chunks
- Backups of similar data share chunks

## Development Status

### Implemented Features

- [x] Core FUSE filesystem operations
- [x] AES-256-GCM encryption with Argon2id KDF
- [x] Content-based chunking and deduplication
- [x] LZ4 compression
- [x] Local LRU caching
- [x] Encrypted metadata storage (sled)
- [x] File versioning
- [x] Snapshot management
- [x] Rate limiting for Telegram API
- [x] Machine identity with Ed25519 signing
- [x] Vector clocks for causality tracking
- [x] Namespace isolation
- [x] Config v2 with YAML support
- [x] Master-replica replication
- [x] CRDT distributed writes
- [x] Conflict detection and resolution
- [x] Access control with ACLs

### In Progress

- [ ] Full Telegram API integration (grammers)
- [ ] Proper daemonization
- [ ] Fsync/durability guarantees
- [ ] Extended attributes
- [ ] Hard links
- [ ] CLI commands for cluster management

## Project Structure

```
src/
├── main.rs           # CLI entry point
├── lib.rs            # Library root
├── config.rs         # Configuration management (v1 + v2)
├── error.rs          # Error types
├── crypto/           # Cryptography module
│   ├── mod.rs
│   ├── kdf.rs        # Argon2id key derivation
│   ├── encryption.rs # AES-256-GCM encryption
│   └── keys.rs       # Key management
├── chunk/            # Chunking and compression
│   ├── mod.rs
│   ├── chunker.rs    # File chunking
│   └── compression.rs# LZ4 compression
├── metadata/         # Filesystem metadata
│   ├── mod.rs
│   ├── inode.rs      # Inode representation
│   ├── store.rs      # Sled-based storage
│   └── version.rs    # File versioning
├── cache/            # Local caching
│   ├── mod.rs
│   └── lru.rs        # LRU implementation
├── telegram/         # Telegram backend
│   ├── mod.rs
│   ├── client.rs     # Telegram API client
│   └── rate_limit.rs # Rate limiting
├── fs/               # FUSE filesystem
│   ├── mod.rs
│   ├── filesystem.rs # Main FUSE implementation
│   └── handle.rs     # File handle management
├── snapshot/         # Snapshot management
│   ├── mod.rs
│   └── snapshot.rs   # Snapshot implementation
└── distributed/      # Distributed infrastructure
    ├── mod.rs        # Module exports
    ├── identity.rs   # Machine identity (Ed25519)
    ├── vector_clock.rs # Vector clock implementation
    ├── namespace.rs  # Namespace management
    ├── types.rs      # Core distributed types
    ├── crdt.rs       # CRDT operations and sync
    ├── replication.rs# Master-replica snapshots
    └── sync.rs       # Sync daemon
```

## Test Coverage

```
121 tests passing
├── crypto/           - Encryption, KDF, key management
├── chunk/            - Chunking, compression, deduplication
├── metadata/         - Inode operations, storage, versioning
├── cache/            - LRU cache operations
├── snapshot/         - Snapshot creation, serialization
├── telegram/         - Rate limiting
└── distributed/      - All distributed features
    ├── identity     (7 tests)
    ├── vector_clock (16 tests)
    ├── namespace    (7 tests)
    ├── types        (7 tests)
    ├── crdt         (5 tests)
    ├── replication  (5 tests)
    └── sync         (6 tests)
```

## License

MIT License - see [LICENSE](LICENSE) for details.

## Contributing

Contributions are welcome! Please read the contributing guidelines before submitting PRs.

## Acknowledgments

- [fuser](https://github.com/cberner/fuser) - Rust FUSE library
- [ring](https://github.com/briansmith/ring) - Cryptography
- [sled](https://github.com/spacejam/sled) - Embedded database
- [grammers](https://github.com/Lonami/grammers) - Telegram client library
