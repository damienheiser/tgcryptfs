# Time Machine Implementation Plan

## Overview

To make tgcryptfs work as a Time Machine destination, we need to implement several FUSE operations and create a sparsebundle management layer.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      TIME MACHINE                            │
│                         ↓                                    │
│              Sparsebundle (APFS inside)                      │
│                         ↓                                    │
│    ┌────────────────────────────────────────────────────┐   │
│    │              tgcryptfs FUSE Layer                   │   │
│    │  ┌──────────────────────────────────────────────┐  │   │
│    │  │  Extended Attributes (setxattr/getxattr)     │  │   │
│    │  │  Resource Forks (._* files)                  │  │   │
│    │  │  Hard Links (link/unlink tracking)           │  │   │
│    │  │  Large File Support (sparse files)           │  │   │
│    │  └──────────────────────────────────────────────┘  │   │
│    └────────────────────────────────────────────────────┘   │
│                         ↓                                    │
│              Telegram Cloud Storage                          │
└─────────────────────────────────────────────────────────────┘
```

## Phase 1: Extended Attributes (Required)

### FUSE Operations to Implement

```rust
// In src/fs/fuse.rs or src/fs/overlay/filesystem.rs

fn setxattr(
    &mut self,
    _req: &Request,
    ino: u64,
    name: &OsStr,
    value: &[u8],
    flags: i32,
    position: u32,
    reply: ReplyEmpty,
) {
    // Store xattr in metadata database
    // Key: (inode, name) -> value
}

fn getxattr(
    &mut self,
    _req: &Request,
    ino: u64,
    name: &OsStr,
    size: u32,
    reply: ReplyXattr,
) {
    // Retrieve xattr from metadata database
}

fn listxattr(
    &mut self,
    _req: &Request,
    ino: u64,
    size: u32,
    reply: ReplyXattr,
) {
    // List all xattr names for inode
}

fn removexattr(
    &mut self,
    _req: &Request,
    ino: u64,
    name: &OsStr,
    reply: ReplyEmpty,
) {
    // Remove xattr from metadata database
}
```

### Metadata Schema Extension

```rust
// In src/metadata/xattr.rs

pub struct XattrStore {
    db: sled::Tree,
}

impl XattrStore {
    pub fn set(&self, inode: u64, name: &str, value: &[u8]) -> Result<()>;
    pub fn get(&self, inode: u64, name: &str) -> Result<Option<Vec<u8>>>;
    pub fn list(&self, inode: u64) -> Result<Vec<String>>;
    pub fn remove(&self, inode: u64, name: &str) -> Result<()>;
}
```

## Phase 2: Hard Links (Required for Deduplication)

Time Machine uses hard links extensively for space-efficient backups.

```rust
// Hard link tracking in metadata

pub struct HardLinkStore {
    // inode -> set of paths pointing to it
    links: sled::Tree,
    // path -> inode (for reverse lookup)
    paths: sled::Tree,
}

fn link(
    &mut self,
    _req: &Request,
    ino: u64,
    newparent: u64,
    newname: &OsStr,
    reply: ReplyEntry,
) {
    // Create new directory entry pointing to existing inode
    // Increment link count
}
```

## Phase 3: Sparsebundle Support

### Option A: Native Sparsebundle Handling

Create sparsebundle directly on tgcryptfs:

```rust
// CLI command
tgcryptfs timemachine create --size 500g --name "MacBackup"

// Creates:
// /mount/MacBackup.sparsebundle/
//   Info.plist
//   bands/
//     0, 1, 2, ... (8MB band files)
//   token
```

### Option B: Local Sparsebundle with Sync

```bash
# Create locally
hdiutil create -size 500g -type SPARSEBUNDLE -fs APFS \
  -volname "TM-Backup" ~/tm-backup.sparsebundle

# Mount sparsebundle
hdiutil attach ~/tm-backup.sparsebundle -mountpoint /Volumes/TM-Backup

# Set as Time Machine destination
sudo tmutil setdestination /Volumes/TM-Backup

# Sync bands to tgcryptfs (incremental)
rsync -avh ~/tm-backup.sparsebundle/ ~/mnt/tgcryptfs/tm-backup.sparsebundle/
```

## Phase 4: CLI Commands

```bash
# Initialize Time Machine support
tgcryptfs timemachine init --size 500g

# Mount Time Machine volume
tgcryptfs timemachine mount

# Show Time Machine status
tgcryptfs timemachine status

# Sync local sparsebundle to cloud
tgcryptfs timemachine sync
```

## Implementation Priority

1. **High Priority (Phase 1)**
   - [ ] `setxattr` / `getxattr` / `listxattr` / `removexattr`
   - [ ] XattrStore in metadata database
   - Estimated: 2-3 days

2. **Medium Priority (Phase 2)**
   - [ ] Hard link support (`link` operation)
   - [ ] Link count tracking
   - Estimated: 2-3 days

3. **Lower Priority (Phase 3-4)**
   - [ ] Sparsebundle band file optimization
   - [ ] CLI commands for Time Machine management
   - Estimated: 1 week

## Testing

```bash
# Test xattr support
xattr -w com.apple.test "value" /mnt/tgcryptfs/testfile
xattr -p com.apple.test /mnt/tgcryptfs/testfile

# Test hard links
ln /mnt/tgcryptfs/file1 /mnt/tgcryptfs/file2
stat /mnt/tgcryptfs/file1  # Should show 2 links

# Test Time Machine compatibility
sudo tmutil setdestination /mnt/tgcryptfs/tm-backup
tmutil startbackup
```

## Current Workaround

Until full implementation, use rsync-based backup:

```bash
# Mount tgcryptfs
tgcryptfs mount ~/mnt/tgcryptfs --password-file ~/.tgcryptfs-key

# Sync home directory (excludes system files)
rsync -avh --delete \
  --exclude-from=~/.config/tgcryptfs/excludes.txt \
  ~/ ~/mnt/tgcryptfs/home/
```

This provides:
- Encrypted cloud backup
- Incremental sync
- Cross-platform compatibility
- No special filesystem requirements
