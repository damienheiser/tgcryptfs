# Overlay Filesystem Research for tgcryptfs

## Executive Summary

This document synthesizes research on overlay filesystem implementations across Linux kernel, FUSE, and macOS platforms to inform the design of an overlay mode for tgcryptfs.

## 1. Linux OverlayFS Kernel Implementation

### Architecture
Linux OverlayFS (kernel 3.18+) combines "upper" (writable) and "lower" (read-only) directories into a merged view.

```
┌──────────────────────────────────────┐
│     Merged Filesystem View           │
└──────────────────────────────────────┘
         │                │
    ┌─────────┐      ┌──────────┐
    │ Upper   │      │ Lower    │
    │(RW)     │      │(RO)      │
    └─────────┘      └──────────┘
```

### Whiteouts
**Character Device (0/0)**: Fast, simple, well-tested. Created when file deleted to hide lower layer entry.

**Extended Attribute**: `user.overlay.whiteout` - More compatible, supports nesting.

### Opaque Directories
Mark with xattr `user.overlay.opaque=y` to hide all lower layer entries completely.

### Copy-Up
Full file copied to upper on first write. Blocks until complete. No lazy/partial copy in kernel version.

## 2. FUSE Implementations

### fuse-overlayfs
- Production-ready, used by Podman/containerd
- ~17% overhead on SSD
- Supports UID/GID mapping

### unionfs-fuse
- Simpler but macOS Finder integration broken
- Works for command-line only

## 3. macOS Limitations

- Native union mounts: Broken in Sonoma 14.4.1+
- unionfs-fuse: Finder fails with error -50
- **Recommendation**: Use sync-based approach on macOS

## 4. Recommended Approach for tgcryptfs

### Linux
Use native kernel OverlayFS with:
- Read-only FUSE mount as lower layer
- Local upper directory for modifications
- Character device whiteouts

### macOS
Implement sync-based overlay:
- rsync-based background sync
- Virtual overlay in application layer

## References
- [Linux OverlayFS Docs](https://kernel.org/doc/html/latest/filesystems/overlayfs.html)
- [fuse-overlayfs](https://github.com/containers/fuse-overlayfs)
