//! Overlay FUSE filesystem implementation
//!
//! Combines upper (tgcryptfs) and lower (local) layers into a merged view.

use fuser::{
    Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, Request,
};
use libc::{ENOENT, ENOTDIR, EROFS};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, error};

use super::{
    handle::OverlayHandleManager,
    inode::{InodeSource, OverlayAttributes, OverlayFileType, OverlayInode, OverlayInodeManager},
    lower::LowerLayer,
    whiteout::WhiteoutStore,
    OverlayConfig,
};

const TTL: Duration = Duration::from_secs(1);

/// Overlay FUSE filesystem
pub struct OverlayFs {
    /// Configuration
    config: OverlayConfig,
    /// Lower layer (local filesystem)
    lower: LowerLayer,
    /// Whiteout tracking
    whiteouts: WhiteoutStore,
    /// Virtual inode management
    inodes: OverlayInodeManager,
    /// File handle manager
    handles: OverlayHandleManager,
    /// UID
    uid: u32,
    /// GID
    gid: u32,
}

impl OverlayFs {
    /// Create a new overlay filesystem
    pub fn new(config: OverlayConfig) -> crate::error::Result<Self> {
        let lower = LowerLayer::new(config.lower_path.clone(), config.clone())?;
        let whiteouts = WhiteoutStore::open(&config.whiteout_db_path)?;

        Ok(Self {
            config,
            lower,
            whiteouts,
            inodes: OverlayInodeManager::new(),
            handles: OverlayHandleManager::new(),
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        })
    }

    /// Get virtual path from parent inode and name
    fn get_path(&self, parent: u64, name: &OsStr) -> Option<PathBuf> {
        let parent_inode = self.inodes.get(parent)?;
        Some(parent_inode.path.join(name))
    }

    /// Lookup or create inode for a path
    fn lookup_inode(&self, parent: u64, name: &OsStr) -> Option<OverlayInode> {
        let path = self.get_path(parent, name)?;

        // Check if already cached
        if let Some(inode) = self.inodes.get_by_path(&path) {
            return Some(inode);
        }

        // Check whiteout
        if self.whiteouts.is_whiteout(&path) {
            return None;
        }

        // Check if parent is opaque (hide lower layer)
        let parent_inode = self.inodes.get(parent)?;
        let check_lower = !self.whiteouts.is_opaque(&parent_inode.path);

        // TODO: Check upper layer first when integrated

        // Check lower layer
        if check_lower && self.lower.exists(&path) {
            let meta = self.lower.metadata(&path).ok()?;
            let ino = self.inodes.alloc_ino();
            let inode = OverlayInode::from_lower(
                ino,
                parent,
                name.to_string_lossy().to_string(),
                path.clone(),
                &meta,
            );
            self.inodes.register(inode.clone());
            return Some(inode);
        }

        None
    }

    /// Read directory entries, merging upper and lower
    fn read_merged_dir(&self, ino: u64) -> Vec<(String, OverlayFileType, u64)> {
        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let dir_inode = match self.inodes.get(ino) {
            Some(i) => i,
            None => return entries,
        };

        let dir_path = &dir_inode.path;

        // Get whiteouts for this directory
        let whiteouts = self.whiteouts.whiteouts_in_dir(dir_path);

        // Check if directory is opaque
        let is_opaque = self.whiteouts.is_opaque(dir_path);

        // TODO: Add upper layer entries first when integrated

        // Add lower layer entries (if not opaque)
        if !is_opaque {
            if let Some(lower_path) = &dir_inode.lower_path {
                if let Ok(lower_entries) = self.lower.readdir(lower_path) {
                    for entry in lower_entries {
                        let name = entry.name.to_string_lossy().to_string();

                        // Skip if whiteout exists or already seen
                        if whiteouts.contains(&entry.name) || seen.contains(&name) {
                            continue;
                        }

                        // Skip excluded patterns
                        let entry_path = dir_path.join(&entry.name);
                        if self.config.is_excluded(&entry_path) {
                            continue;
                        }

                        seen.insert(name.clone());

                        // Get or create inode
                        let child_ino = if let Some(child) = self.inodes.get_by_path(&entry_path) {
                            child.ino
                        } else {
                            // Create placeholder inode
                            let ino = self.inodes.alloc_ino();
                            if let Ok(meta) = self.lower.metadata(&entry_path) {
                                let child = OverlayInode::from_lower(
                                    ino,
                                    dir_inode.ino,
                                    name.clone(),
                                    entry_path,
                                    &meta,
                                );
                                self.inodes.register(child);
                            }
                            ino
                        };

                        let file_type = OverlayFileType::from(entry.file_type);
                        entries.push((name, file_type, child_ino));
                    }
                }
            }
        }

        entries
    }
}

impl Filesystem for OverlayFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        debug!("lookup(parent={}, name={:?})", parent, name);

        match self.lookup_inode(parent, name) {
            Some(inode) => {
                let attr = inode.to_fuser_attr();
                reply.entry(&TTL, &attr, 0);
            }
            None => {
                reply.error(ENOENT);
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr(ino={})", ino);

        match self.inodes.get(ino) {
            Some(inode) => {
                // Refresh attributes from source
                let attr = match inode.source {
                    InodeSource::Lower => {
                        if let Some(ref path) = inode.lower_path {
                            if let Ok(meta) = self.lower.metadata(path) {
                                let mut updated = inode.clone();
                                updated.attrs = OverlayAttributes::from_metadata(&meta);
                                self.inodes.update(ino, updated.clone());
                                updated.to_fuser_attr()
                            } else {
                                inode.to_fuser_attr()
                            }
                        } else {
                            inode.to_fuser_attr()
                        }
                    }
                    _ => inode.to_fuser_attr(),
                };
                reply.attr(&TTL, &attr);
            }
            None => {
                reply.error(ENOENT);
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir(ino={}, offset={})", ino, offset);

        let inode = match self.inodes.get(ino) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        if inode.file_type != OverlayFileType::Directory {
            reply.error(ENOTDIR);
            return;
        }

        let mut entries: Vec<(String, OverlayFileType, u64)> = vec![
            (".".to_string(), OverlayFileType::Directory, ino),
            ("..".to_string(), OverlayFileType::Directory, inode.parent),
        ];

        entries.extend(self.read_merged_dir(ino));

        for (i, (name, file_type, child_ino)) in entries.iter().enumerate().skip(offset as usize) {
            let buffer_full = reply.add(
                *child_ino,
                (i + 1) as i64,
                file_type.to_fuser_type(),
                name,
            );
            if buffer_full {
                break;
            }
        }

        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("open(ino={}, flags={})", ino, flags);

        let inode = match self.inodes.get(ino) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Check if trying to write to lower layer
        let accmode = flags & libc::O_ACCMODE;
        let wants_write = accmode == libc::O_WRONLY || accmode == libc::O_RDWR;

        if wants_write && inode.source == InodeSource::Lower {
            // TODO: Implement copy-up to upper layer
            // For now, return read-only error
            reply.error(EROFS);
            return;
        }

        let fh = self.handles.open(ino, inode.source, flags);

        // Set lower path if from lower layer
        if inode.source == InodeSource::Lower {
            if let Some(ref path) = inode.lower_path {
                self.handles.set_lower_path(fh, path.clone());
            }
        }

        reply.opened(fh, 0);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read(ino={}, fh={}, offset={}, size={})", ino, fh, offset, size);

        let handle = match self.handles.get(fh) {
            Some(h) => h,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match handle.source {
            InodeSource::Lower => {
                if let Some(ref path) = handle.lower_path {
                    match self.lower.read(path, offset as u64, size) {
                        Ok(data) => reply.data(&data),
                        Err(e) => {
                            error!("Failed to read from lower layer: {}", e);
                            reply.error(libc::EIO);
                        }
                    }
                } else {
                    reply.error(ENOENT);
                }
            }
            InodeSource::Upper => {
                // TODO: Read from upper layer (tgcryptfs)
                reply.error(ENOENT);
            }
            InodeSource::Merged => {
                // Shouldn't happen for files
                reply.error(ENOENT);
            }
        }
    }

    fn release(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        debug!("release(ino={}, fh={})", ino, fh);
        self.handles.close(fh);
        reply.ok();
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        debug!("readlink(ino={})", ino);

        let inode = match self.inodes.get(ino) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        if inode.file_type != OverlayFileType::Symlink {
            reply.error(libc::EINVAL);
            return;
        }

        match inode.source {
            InodeSource::Lower => {
                if let Some(ref path) = inode.lower_path {
                    match self.lower.readlink(path) {
                        Ok(target) => reply.data(target.as_os_str().as_encoded_bytes()),
                        Err(e) => {
                            error!("Failed to read symlink: {}", e);
                            reply.error(libc::EIO);
                        }
                    }
                } else {
                    reply.error(ENOENT);
                }
            }
            _ => {
                // TODO: Handle upper layer symlinks
                reply.error(ENOENT);
            }
        }
    }

    fn access(&mut self, _req: &Request, ino: u64, mask: i32, reply: ReplyEmpty) {
        debug!("access(ino={}, mask={})", ino, mask);

        if self.inodes.exists(ino) {
            // For now, allow all access to existing inodes
            // TODO: Proper permission checking
            reply.ok();
        } else {
            reply.error(ENOENT);
        }
    }

    fn statfs(&mut self, _req: &Request, _ino: u64, reply: fuser::ReplyStatfs) {
        // Return stats from lower layer
        reply.statfs(
            1000000,        // blocks
            500000,         // bfree
            500000,         // bavail
            1000000,        // files
            500000,         // ffree
            4096,           // bsize
            255,            // namelen
            4096,           // frsize
        );
    }
}
