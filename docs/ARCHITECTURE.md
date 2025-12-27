# tgcryptfs Architecture

This document describes the internal architecture of tgcryptfs, a FUSE-based encrypted filesystem backed by Telegram's Saved Messages.

## System Overview

tgcryptfs is structured as a layered system where each layer has clear responsibilities:

```
┌─────────────────────────────────────────────────────────────────┐
│                        CLI Interface                             │
│                         (main.rs)                                │
└─────────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────────┐
│                     FUSE Filesystem Layer                        │
│                      (fs/filesystem.rs)                          │
│                                                                  │
│  Translates POSIX operations → tgcryptfs operations            │
└─────────────────────────────────────────────────────────────────┘
         │                    │                    │
         ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│   Metadata      │  │     Cache       │  │    Telegram     │
│    Store        │  │     Layer       │  │    Backend      │
│  (sled + enc)   │  │  (disk LRU)     │  │   (grammers)    │
└─────────────────┘  └─────────────────┘  └─────────────────┘
         │                    │                    │
         └────────────────────┼────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Crypto Layer                                │
│                                                                  │
│  Key Management │ AES-256-GCM │ Argon2id KDF │ BLAKE3 Hashing  │
└─────────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. CLI Interface (`main.rs`)

The entry point handles command parsing and orchestrates the major operations:

- **init**: Creates configuration and data directories
- **auth**: Handles Telegram authentication flow
- **mount**: Sets up and mounts the FUSE filesystem
- **unmount**: Cleanly unmounts the filesystem
- **snapshot/restore**: Manages filesystem snapshots
- **cache**: Cache management operations
- **sync**: Synchronization with Telegram

### 2. Configuration (`config.rs`)

Manages all configurable aspects of the system:

```rust
pub struct Config {
    pub telegram: TelegramConfig,     // API credentials, concurrency
    pub encryption: EncryptionConfig, // Argon2 parameters, salt
    pub cache: CacheConfig,           // Size, prefetch settings
    pub chunk: ChunkConfig,           // Chunk size, compression
    pub mount: MountConfig,           // Mount options
    pub versioning: VersioningConfig, // Version history settings
    pub data_dir: PathBuf,            // Local data directory
}
```

### 3. Crypto Module (`crypto/`)

Provides all cryptographic operations:

#### Key Derivation (`kdf.rs`)
```
Password ──► Argon2id(salt, params) ──► 256-bit Master Key
```

Parameters are configurable:
- `argon2_memory_kib`: Memory cost (default 64MB)
- `argon2_iterations`: Time cost (default 3)
- `argon2_parallelism`: Parallelism (default 4)

#### Key Management (`keys.rs`)
```
Master Key
    │
    ├──► HKDF("tgcryptfs-metadata-v1") ──► Metadata Key
    │
    └──► HKDF("tgcryptfs-chunk-v1:<chunk_id>") ──► Per-Chunk Key
```

Each chunk gets a unique encryption key derived from the master key and chunk ID, providing key separation.

#### Encryption (`encryption.rs`)
- Algorithm: AES-256-GCM
- Nonce: 12 bytes, randomly generated per encryption
- Tag: 16 bytes, appended to ciphertext
- AAD: Optional additional authenticated data

### 4. Chunk Module (`chunk/`)

Handles file splitting and content-addressing:

#### Chunker (`chunker.rs`)
- Fixed-size chunking (default 50MB, max 2GB for Telegram limit)
- Content hashing with BLAKE3 for deduplication
- Chunk reassembly preserving order

#### Chunk Reference
```rust
pub struct ChunkRef {
    pub id: ChunkId,        // BLAKE3 hash (content-based)
    pub size: u64,          // Encrypted size
    pub message_id: i32,    // Telegram message ID
    pub offset: u64,        // Offset in original file
    pub original_size: u64, // Uncompressed size
    pub compressed: bool,   // Compression applied?
}
```

#### Compression (`compression.rs`)
- Algorithm: LZ4 (fast compression/decompression)
- Threshold: Only compress if > 1KB
- Decision: Only use if result is smaller

### 5. Metadata Module (`metadata/`)

Stores and manages filesystem metadata:

#### Inode (`inode.rs`)
POSIX-compatible inode representation:
```rust
pub struct Inode {
    pub ino: u64,                    // Inode number
    pub parent: u64,                 // Parent inode
    pub name: String,                // File/directory name
    pub attrs: InodeAttributes,      // POSIX attributes
    pub manifest: Option<ChunkManifest>, // File chunks
    pub symlink_target: Option<String>,  // Symlink destination
    pub children: Vec<u64>,          // Directory children
    pub version: u64,                // Current version
    pub xattrs: HashMap<String, Vec<u8>>, // Extended attributes
}
```

#### Metadata Store (`store.rs`)
Encrypted sled-based database with:
- **Inode storage**: Encrypted inode data
- **Parent-name index**: Fast lookups by path
- **Chunk references**: Reference counting for dedup
- **In-memory cache**: Reduce database reads

Data layout:
```
inodes/         ino (8 bytes) → encrypted(Inode)
parent_index/   parent + name → ino
chunks/         chunk_id → message_id + ref_count
metadata/       key → encrypted(value)
```

#### Version Manager (`version.rs`)
Tracks file history:
```rust
pub struct FileVersion {
    pub version: u64,
    pub created: SystemTime,
    pub size: u64,
    pub manifest: ChunkManifest,
    pub comment: Option<String>,
}
```

### 6. Cache Module (`cache/`)

Disk-based LRU cache for decrypted chunks:

```rust
pub struct ChunkCache {
    cache_dir: PathBuf,        // Cache directory
    max_size: u64,             // Maximum cache size
    current_size: AtomicU64,   // Current usage
    lru: LruCache<String>,     // LRU tracking
    prefetch_queue: VecDeque<String>, // Prefetch queue
}
```

Operations:
- **get**: Retrieve cached chunk, update LRU
- **put**: Cache chunk, evict if necessary
- **remove**: Explicitly remove chunk
- **queue_prefetch**: Queue chunks for background prefetch

### 7. Telegram Module (`telegram/`)

Handles all Telegram API communication:

#### Client (`client.rs`)
```rust
pub struct TelegramBackend {
    config: TelegramConfig,
    upload_limiter: RateLimiter,
    download_limiter: RateLimiter,
    // ... connection state
}
```

Operations:
- **connect/disconnect**: Session management
- **upload_chunk**: Upload encrypted data as document
- **download_chunk**: Download by message ID
- **delete_message**: Remove orphaned chunks
- **list_chunks**: Enumerate stored chunks

#### Rate Limiter (`rate_limit.rs`)
Token bucket rate limiting with:
- Concurrency limits (semaphore-based)
- Request rate limiting (token bucket)
- Exponential backoff for retries

### 8. Filesystem Module (`fs/`)

FUSE implementation:

#### TgCryptFs (`filesystem.rs`)
Implements `fuser::Filesystem` trait:
- **lookup**: Resolve path component
- **getattr/setattr**: Attribute operations
- **readdir**: List directory contents
- **open/release**: File handle management
- **read/write**: Data operations
- **create/mkdir**: Create files/directories
- **unlink/rmdir**: Remove files/directories
- **rename**: Move/rename operations
- **statfs**: Filesystem statistics

#### File Handle (`handle.rs`)
Manages open file state:
```rust
pub struct FileHandle {
    pub ino: u64,
    pub flags: i32,
    pub write_buffer: Vec<u8>,  // Buffered writes
    pub dirty: bool,            // Uncommitted changes
}
```

### 9. Snapshot Module (`snapshot/`)

Point-in-time filesystem snapshots:

```rust
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub created: DateTime<Utc>,
    pub description: Option<String>,
    pub inodes: HashMap<u64, Vec<u8>>, // Serialized inodes
}
```

Since chunks are immutable and content-addressed, snapshots only need to store inode metadata. Restoration re-links to existing chunks.

## Data Flow

### Write Path

```
Application write()
        │
        ▼
┌───────────────────┐
│  Buffer in handle │
└───────────────────┘
        │ (on release/flush)
        ▼
┌───────────────────┐
│   Split into      │
│   chunks          │
└───────────────────┘
        │
        ▼
┌───────────────────┐     ┌──────────────┐
│ For each chunk:   │────►│ Already in   │──► Skip upload
│ Check dedup       │     │ metadata?    │    (add ref)
└───────────────────┘     └──────────────┘
        │ (new chunk)
        ▼
┌───────────────────┐
│   Compress (LZ4)  │
│   if beneficial   │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│   Encrypt chunk   │
│   (AES-256-GCM)   │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│  Upload to        │
│  Telegram         │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│  Update metadata  │
│  (inode, refs)    │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│  Cache decrypted  │
│  chunk locally    │
└───────────────────┘
```

### Read Path

```
Application read()
        │
        ▼
┌───────────────────┐
│  Get inode from   │
│  metadata store   │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│  Find chunks for  │
│  requested range  │
└───────────────────┘
        │
        ▼
┌───────────────────┐     ┌──────────────┐
│ For each chunk:   │────►│ In local     │──► Return cached
│ Check cache       │     │ cache?       │
└───────────────────┘     └──────────────┘
        │ (cache miss)
        ▼
┌───────────────────┐
│  Download from    │
│  Telegram         │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│   Decrypt chunk   │
│   (AES-256-GCM)   │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│   Decompress      │
│   if compressed   │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│   Cache for       │
│   future reads    │
└───────────────────┘
        │
        ▼
┌───────────────────┐
│  Extract range    │
│  and return       │
└───────────────────┘
```

## Concurrency Model

- **Tokio runtime**: Async operations for Telegram I/O
- **Parking lot locks**: Fast synchronization for metadata/cache
- **Atomic operations**: Lock-free counters and flags
- **Semaphores**: Rate limiting concurrency

## Error Handling

Centralized error type with POSIX errno mapping:

```rust
pub enum Error {
    // Crypto errors → EIO
    Encryption(String),
    Decryption(String),

    // Path errors → ENOENT, ENOTDIR, etc.
    InodeNotFound(u64),
    PathNotFound(String),
    NotADirectory(String),

    // Telegram errors → EIO, EAGAIN
    TelegramClient(String),
    TelegramRateLimited { seconds: u32 },

    // ...
}

impl Error {
    pub fn to_errno(&self) -> c_int { ... }
}
```

## Future Considerations

### Planned Improvements

1. **Delta synchronization**: Only sync changed chunks
2. **Parallel chunk operations**: Concurrent upload/download
3. **Read-ahead prefetching**: Predictive chunk loading
4. **Garbage collection**: Periodic cleanup of orphaned chunks
5. **Offline mode**: Queue operations for later sync

### Scalability Notes

- Metadata stored locally (sled scales to millions of entries)
- Chunk references counted for safe garbage collection
- LRU cache prevents unbounded local storage
- Rate limiting prevents Telegram API abuse
