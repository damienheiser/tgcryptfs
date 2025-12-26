# tgcryptfs

An encrypted FUSE filesystem with cloud backend storage, providing unlimited storage with end-to-end encryption and distributed multi-machine synchronization.

## Overview

tgcryptfs mounts as a standard filesystem on your computer, but all files are:
1. **Encrypted locally** using AES-256-GCM with keys derived from your password
2. **Chunked** into manageable pieces (default 50MB)
3. **Compressed** using LZ4 when beneficial
4. **Deduplicated** using content-addressable storage (BLAKE3 hashes)
5. **Uploaded** to your cloud backend as documents
6. **Synchronized** across multiple machines with conflict resolution

Your data remains encrypted end-to-end — the cloud backend only sees encrypted blobs.

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

### Distributed Features
- **Machine Identity**: Each instance has a unique cryptographic identity (Ed25519)
- **Namespace Isolation**: Multiple independent filesystems on the same account
- **Master-Replica Mode**: One writer, multiple read-only replicas with automatic sync
- **CRDT Distributed Mode**: Full read/write from any node with automatic conflict resolution
- **Vector Clocks**: Causality tracking for distributed operations
- **Access Control**: Per-namespace ACLs with machine-level permissions

## Prerequisites

- Rust 1.70+
- FUSE (libfuse3 on Linux, macFUSE on macOS)
- API credentials from [my.telegram.org](https://my.telegram.org)

## Installation

### Step 1: Get API Credentials

1. Visit [my.telegram.org](https://my.telegram.org)
2. Log in with your phone number
3. Click "API development tools"
4. Create a new application (name can be anything)
5. Note your `api_id` and `api_hash`

### Step 2: Install Dependencies

**Linux (Debian/Ubuntu):**
```bash
sudo apt-get update
sudo apt-get install -y libfuse3-dev pkg-config build-essential
```

**Linux (Fedora):**
```bash
sudo dnf install -y fuse3-devel pkg-config gcc
```

**macOS:**
```bash
brew install macfuse
```

### Step 3: Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### Step 4: Build tgcryptfs

```bash
# Clone the repository
git clone https://github.com/damienheiser/tgcryptfs.git
cd tgcryptfs

# Build release binary
cargo build --release

# Install to system (optional)
sudo cp target/release/tgcryptfs /usr/local/bin/
```

### Step 5: Initial Configuration

```bash
# Create config directory
mkdir -p ~/.config/tgcryptfs

# Initialize with your API credentials
tgcryptfs init --api-id YOUR_API_ID --api-hash YOUR_API_HASH
```

### Step 6: Authenticate

```bash
# Start authentication
tgcryptfs auth --phone +1234567890

# Enter the code sent to your app
# If you have 2FA enabled, you'll be prompted for your password
```

### Step 7: Mount and Use

```bash
# Create mount point
sudo mkdir -p /mnt/tgcryptfs
sudo chown $USER:$USER /mnt/tgcryptfs

# Mount the filesystem
tgcryptfs mount /mnt/tgcryptfs
# Enter your encryption password when prompted

# Use it like a normal filesystem!
cp ~/documents/* /mnt/tgcryptfs/
ls /mnt/tgcryptfs/

# Unmount when done
tgcryptfs unmount /mnt/tgcryptfs
```

## Automated Mounting

For systemd services or scripts, you can use the `--password-file` option:

```bash
# Create a secure password file
echo "your-encryption-password" | sudo tee /etc/tgcryptfs/password > /dev/null
sudo chmod 600 /etc/tgcryptfs/password

# Mount with password file
tgcryptfs mount /mnt/tgcryptfs --password-file /etc/tgcryptfs/password
```

### Systemd Service Example

```ini
# /etc/systemd/system/tgcryptfs.service
[Unit]
Description=tgcryptfs Encrypted Filesystem
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/tgcryptfs mount /mnt/tgcryptfs -f --password-file /etc/tgcryptfs/password
ExecStop=/usr/local/bin/tgcryptfs unmount /mnt/tgcryptfs
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

## Commands

| Command | Description |
|---------|-------------|
| `tgcryptfs init` | Initialize configuration with API credentials |
| `tgcryptfs auth` | Authenticate with the cloud backend |
| `tgcryptfs mount <path>` | Mount the filesystem |
| `tgcryptfs unmount <path>` | Unmount the filesystem |
| `tgcryptfs status` | Show filesystem and connection status |
| `tgcryptfs snapshot <name>` | Create a named snapshot |
| `tgcryptfs snapshots` | List all snapshots |
| `tgcryptfs restore <name>` | Restore from a snapshot |
| `tgcryptfs cache` | Show cache statistics |
| `tgcryptfs cache --clear` | Clear the local cache |
| `tgcryptfs sync` | Sync local state with cloud |

## Distribution Modes

### Standalone (Default)
Single machine, independent filesystem.

```yaml
distribution:
  mode: standalone
```

### Master-Replica
One master (read/write) with multiple replicas (read-only).

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
Full multi-writer support with automatic conflict resolution.

```yaml
distribution:
  mode: distributed
  cluster_id: "home-cluster"
  distributed:
    sync_interval_ms: 1000
    conflict_resolution: last-write-wins
```

## Configuration

Configuration is stored in `~/.config/tgcryptfs/config.json`:

```json
{
  "telegram": {
    "api_id": 12345678,
    "api_hash": "your_api_hash",
    "session_file": "/path/to/session",
    "max_concurrent_uploads": 3,
    "max_concurrent_downloads": 5
  },
  "encryption": {
    "argon2_memory_kib": 65536,
    "argon2_iterations": 3,
    "argon2_parallelism": 4
  },
  "cache": {
    "max_size": 10737418240,
    "cache_dir": "/var/cache/tgcryptfs",
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

### Encryption Details

- **Key Derivation**: Argon2id with configurable memory/time/parallelism
- **Encryption**: AES-256-GCM (authenticated encryption)
- **Chunk Hashing**: BLAKE3 for content-addressing and deduplication
- **Nonce Generation**: Cryptographically random 12-byte nonces
- **Signing**: Ed25519 signatures for distributed operations

### What the Cloud Backend Sees

The backend only stores encrypted blobs with random-looking filenames. It cannot:
- Read your file contents
- See file names or directory structure
- Know how many files you have (only chunk count)
- Correlate chunks to files

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Cloud Backend Storage                             │
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

## How It Works

### Writing a File

1. File data is split into fixed-size chunks (default 50MB)
2. Each chunk is hashed with BLAKE3 for content-addressing
3. Chunks are compressed with LZ4 if compression helps
4. Each chunk is encrypted with a derived per-chunk key
5. Encrypted chunks are uploaded to the cloud backend
6. Metadata (inodes, directory structure) is encrypted and stored locally
7. In distributed mode, CRDT operations are broadcast to other nodes

### Reading a File

1. File metadata is looked up from the encrypted local database
2. Required chunks are identified from the file's manifest
3. Chunks are fetched from local cache or downloaded from backend
4. Chunks are decrypted and decompressed
5. Data is assembled and returned to the application

### Deduplication

Identical content produces identical chunk hashes, so:
- Copying a file doesn't re-upload data
- Modified files only upload changed chunks
- Backups of similar data share chunks

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
- [grammers](https://github.com/Lonami/grammers) - Client library
