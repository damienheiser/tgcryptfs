# Time Machine Integration Research

## Overview

This document covers research into integrating tgcryptfs with macOS Time Machine for cloud-backed backups.

## Time Machine Requirements

Time Machine requires the following filesystem features:

1. **Extended Attributes (xattr)** - For storing file metadata
2. **Resource Forks** - macOS-specific file metadata
3. **Hard Links** - For deduplication between snapshots
4. **ACL Support** - Access Control Lists
5. **APFS/HFS+ Compatibility** - Native Apple filesystem features

## Current Status

### Direct FUSE Mount
- tgcryptfs FUSE mount does NOT currently support all required operations
- `hdiutil create` fails with "Device not configured" on tgcryptfs mount
- Extended attributes are partially supported (._files created)

### Sparsebundle Approach
The typical approach for network Time Machine destinations:

1. Create an APFS/HFS+ sparsebundle disk image
2. Store the sparsebundle on the network/cloud storage
3. Mount the sparsebundle locally
4. Point Time Machine to the mounted volume

**Issue**: Creating sparsebundle directly on tgcryptfs fails due to missing FUSE operations.

## Workarounds

### Option 1: Local Sparsebundle with Cloud Sync
```bash
# Create sparsebundle locally
hdiutil create -size 500g -type SPARSEBUNDLE -fs "APFS" \
  -volname "TGCryptFS-TM" ~/TimeMachine.sparsebundle

# Mount it
hdiutil attach ~/TimeMachine.sparsebundle

# Set as Time Machine destination
sudo tmutil setdestination /Volumes/TGCryptFS-TM

# Sync sparsebundle to tgcryptfs periodically
rsync -avh ~/TimeMachine.sparsebundle/ ~/mnt/tgcryptfs/TimeMachine.sparsebundle/
```

### Option 2: Rsync-based Backup (Current Implementation)
```bash
# Direct rsync of home directory to tgcryptfs
rsync -avh --delete --exclude-from=excludes.txt ~/ ~/mnt/tgcryptfs/home/
```

This provides:
- Incremental backups
- Encryption via tgcryptfs
- Cloud storage via Telegram
- Cross-platform compatibility

### Option 3: Future FUSE Enhancement
To support Time Machine directly, tgcryptfs would need to implement:

1. Full xattr support in FUSE operations
2. Resource fork handling
3. Hard link support for deduplication
4. Apple-specific ioctl operations

## Implementation Roadmap

### Phase 1: Enhanced xattr Support
- [ ] Implement `setxattr` / `getxattr` / `listxattr` / `removexattr`
- [ ] Store xattrs in metadata database
- [ ] Handle Apple-specific xattr namespaces

### Phase 2: Sparsebundle Support
- [ ] Support large file operations needed by hdiutil
- [ ] Implement proper FUSE flush/fsync operations
- [ ] Handle band file operations

### Phase 3: Time Machine CLI
```bash
# Future CLI commands
tgcryptfs timemachine init --size 500g
tgcryptfs timemachine mount
tgcryptfs timemachine status
```

## References

- [Apple Time Machine Technical Guide](https://support.apple.com/guide/mac-help/back-up-files-mh35860/mac)
- [sparsebundlefs FUSE](https://github.com/torarnv/sparsebundlefs)
- [Backing up to network storage](https://teamdynamix.umich.edu/TDClient/47/LSAPortal/KB/ArticleDet?ID=1840)

## Current Recommendation

Until full Time Machine support is implemented, use **Option 2 (rsync-based backup)** which provides:
- Encrypted cloud backup of home directory
- Incremental sync support
- Works across macOS and Linux
- No special filesystem requirements
