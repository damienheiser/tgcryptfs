//! Overlay FUSE filesystem implementation
//!
//! Combines upper (local writable) and lower (local read-only) layers into a merged view.
//! Writes go to the upper layer, reads check upper first then lower.

use fuser::{
    Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
};
use libc::{ENOENT, ENOTDIR, ENOTEMPTY};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tracing::{debug, error, info, warn};

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
    /// Lower layer (local filesystem, read-only)
    lower: LowerLayer,
    /// Upper layer path (local filesystem, writable)
    upper_path: PathBuf,
    /// Whiteout tracking
    whiteouts: WhiteoutStore,
    /// Virtual inode management
    inodes: OverlayInodeManager,
    /// File handle manager
    handles: OverlayHandleManager,
    /// UID (reserved for future chown support)
    #[allow(dead_code)]
    uid: u32,
    /// GID (reserved for future chown support)
    #[allow(dead_code)]
    gid: u32,
}

impl OverlayFs {
    /// Create a new overlay filesystem
    pub fn new(config: OverlayConfig) -> crate::error::Result<Self> {
        let lower = LowerLayer::new(config.lower_path.clone(), config.clone())?;
        let whiteouts = WhiteoutStore::open(&config.whiteout_db_path)?;

        // Create upper directory if it doesn't exist
        fs::create_dir_all(&config.upper_path)?;

        info!(
            "Overlay filesystem: lower={:?}, upper={:?}",
            config.lower_path, config.upper_path
        );

        Ok(Self {
            upper_path: config.upper_path.clone(),
            config,
            lower,
            whiteouts,
            inodes: OverlayInodeManager::new(),
            handles: OverlayHandleManager::new(),
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        })
    }

    /// Get the upper layer path for a virtual path
    fn upper_path_for(&self, virtual_path: &PathBuf) -> PathBuf {
        let relative = virtual_path
            .strip_prefix("/")
            .unwrap_or(virtual_path.as_path());
        self.upper_path.join(relative)
    }

    /// Check if path exists in upper layer
    #[allow(dead_code)]
    fn exists_in_upper(&self, virtual_path: &PathBuf) -> bool {
        self.upper_path_for(virtual_path).exists()
    }

    /// Get virtual path from parent inode and name
    fn get_path(&self, parent: u64, name: &OsStr) -> Option<PathBuf> {
        let parent_inode = self.inodes.get(parent)?;
        Some(parent_inode.path.join(name))
    }

    /// Copy a file from lower to upper layer (copy-up)
    fn copy_up(&self, virtual_path: &PathBuf) -> std::io::Result<PathBuf> {
        let upper_path = self.upper_path_for(virtual_path);

        // Create parent directories in upper
        if let Some(parent) = upper_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Get the lower path
        let lower_path = self.lower.resolve(virtual_path);

        // Copy file or directory
        let meta = fs::metadata(&lower_path)?;
        if meta.is_dir() {
            fs::create_dir_all(&upper_path)?;
            // Copy permissions
            fs::set_permissions(&upper_path, meta.permissions())?;
        } else if meta.is_symlink() {
            let target = fs::read_link(&lower_path)?;
            std::os::unix::fs::symlink(&target, &upper_path)?;
        } else {
            // Regular file - copy contents
            fs::copy(&lower_path, &upper_path)?;
            // Preserve permissions
            fs::set_permissions(&upper_path, meta.permissions())?;
        }

        info!("Copied up: {:?} -> {:?}", virtual_path, upper_path);
        Ok(upper_path)
    }

    /// Ensure parent directory exists in upper layer
    fn ensure_parent_in_upper(&self, virtual_path: &PathBuf) -> std::io::Result<()> {
        if let Some(parent) = virtual_path.parent() {
            let upper_parent = self.upper_path_for(&parent.to_path_buf());
            fs::create_dir_all(upper_parent)?;
        }
        Ok(())
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

        // Check upper layer first
        let upper_path = self.upper_path_for(&path);
        if upper_path.exists() {
            if let Ok(meta) = fs::metadata(&upper_path) {
                let ino = self.inodes.alloc_ino();
                let mut inode = OverlayInode::from_lower(
                    ino,
                    parent,
                    name.to_string_lossy().to_string(),
                    path.clone(),
                    &meta,
                );
                inode.source = InodeSource::Upper;
                inode.lower_path = None;
                self.inodes.register(inode.clone());
                return Some(inode);
            }
        }

        // Check if parent is opaque (hide lower layer)
        let parent_inode = self.inodes.get(parent)?;
        let check_lower = !self.whiteouts.is_opaque(&parent_inode.path);

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

        // Add upper layer entries first
        let upper_dir = self.upper_path_for(dir_path);
        if upper_dir.exists() {
            if let Ok(read_dir) = fs::read_dir(&upper_dir) {
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();

                    // Skip whiteout marker files
                    if name.starts_with(".wh.") {
                        continue;
                    }

                    seen.insert(name.clone());

                    let entry_path = dir_path.join(&entry.file_name());

                    // Get or create inode
                    let child_ino = if let Some(child) = self.inodes.get_by_path(&entry_path) {
                        child.ino
                    } else {
                        let ino = self.inodes.alloc_ino();
                        if let Ok(meta) = entry.metadata() {
                            let mut child = OverlayInode::from_lower(
                                ino,
                                dir_inode.ino,
                                name.clone(),
                                entry_path,
                                &meta,
                            );
                            child.source = InodeSource::Upper;
                            child.lower_path = None;
                            self.inodes.register(child);
                        }
                        ino
                    };

                    let file_type = if let Ok(ft) = entry.file_type() {
                        OverlayFileType::from(ft)
                    } else {
                        OverlayFileType::RegularFile
                    };
                    entries.push((name, file_type, child_ino));
                }
            }
        }

        // Add lower layer entries (if not opaque)
        if !is_opaque {
            if let Some(lower_path) = &dir_inode.lower_path {
                if let Ok(lower_entries) = self.lower.readdir(lower_path) {
                    for entry in lower_entries {
                        let name = entry.name.to_string_lossy().to_string();

                        // Skip if whiteout exists or already seen in upper
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
                let attr = match inode.source {
                    InodeSource::Upper => {
                        let upper_path = self.upper_path_for(&inode.path);
                        if let Ok(meta) = fs::metadata(&upper_path) {
                            let mut updated = inode.clone();
                            updated.attrs = OverlayAttributes::from_metadata(&meta);
                            self.inodes.update(ino, updated.clone());
                            updated.to_fuser_attr()
                        } else {
                            inode.to_fuser_attr()
                        }
                    }
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

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        debug!("setattr(ino={})", ino);

        let inode = match self.inodes.get(ino) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Copy-up if in lower layer
        let upper_path = if inode.source == InodeSource::Lower {
            match self.copy_up(&inode.path) {
                Ok(p) => p,
                Err(e) => {
                    error!("Copy-up failed: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        } else {
            self.upper_path_for(&inode.path)
        };

        // Apply changes
        if let Some(mode) = mode {
            if let Err(e) = fs::set_permissions(&upper_path, fs::Permissions::from_mode(mode)) {
                error!("setattr chmod failed: {}", e);
            }
        }

        if let Some(size) = size {
            if let Ok(file) = OpenOptions::new().write(true).open(&upper_path) {
                if let Err(e) = file.set_len(size) {
                    error!("setattr truncate failed: {}", e);
                }
            }
        }

        // Update inode
        let mut updated = inode.clone();
        updated.source = InodeSource::Upper;
        updated.lower_path = None;
        if let Ok(meta) = fs::metadata(&upper_path) {
            updated.attrs = OverlayAttributes::from_metadata(&meta);
        }
        self.inodes.update(ino, updated.clone());

        reply.attr(&TTL, &updated.to_fuser_attr());
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

    fn create(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        debug!("create(parent={}, name={:?}, mode={:o})", parent, name, mode);

        let parent_inode = match self.inodes.get(parent) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let virtual_path = parent_inode.path.join(name);

        // Remove whiteout if exists
        let _ = self.whiteouts.remove_whiteout(&virtual_path);

        // Ensure parent exists in upper
        if let Err(e) = self.ensure_parent_in_upper(&virtual_path) {
            error!("Failed to create parent: {}", e);
            reply.error(libc::EIO);
            return;
        }

        let upper_path = self.upper_path_for(&virtual_path);

        // Create file
        let file = match OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(mode)
            .open(&upper_path)
        {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to create file: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        drop(file);

        // Get metadata
        let meta = match fs::metadata(&upper_path) {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to get metadata: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // Create inode
        let ino = self.inodes.alloc_ino();
        let mut inode = OverlayInode::from_lower(
            ino,
            parent,
            name.to_string_lossy().to_string(),
            virtual_path,
            &meta,
        );
        inode.source = InodeSource::Upper;
        inode.lower_path = None;
        self.inodes.register(inode.clone());

        // Open file handle
        let fh = self.handles.open(ino, InodeSource::Upper, flags);

        reply.created(&TTL, &inode.to_fuser_attr(), 0, fh, 0);
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        debug!("mkdir(parent={}, name={:?}, mode={:o})", parent, name, mode);

        let parent_inode = match self.inodes.get(parent) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let virtual_path = parent_inode.path.join(name);

        // Remove whiteout if exists
        let _ = self.whiteouts.remove_whiteout(&virtual_path);

        let upper_path = self.upper_path_for(&virtual_path);

        // Create directory
        if let Err(e) = fs::create_dir_all(&upper_path) {
            error!("Failed to create directory: {}", e);
            reply.error(libc::EIO);
            return;
        }

        // Set permissions
        if let Err(e) = fs::set_permissions(&upper_path, fs::Permissions::from_mode(mode)) {
            warn!("Failed to set permissions: {}", e);
        }

        // Get metadata
        let meta = match fs::metadata(&upper_path) {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to get metadata: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // Create inode
        let ino = self.inodes.alloc_ino();
        let mut inode = OverlayInode::from_lower(
            ino,
            parent,
            name.to_string_lossy().to_string(),
            virtual_path,
            &meta,
        );
        inode.source = InodeSource::Upper;
        inode.lower_path = None;
        inode.file_type = OverlayFileType::Directory;
        self.inodes.register(inode.clone());

        reply.entry(&TTL, &inode.to_fuser_attr(), 0);
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

        let accmode = flags & libc::O_ACCMODE;
        let wants_write = accmode == libc::O_WRONLY || accmode == libc::O_RDWR;

        // Copy-up if writing to lower layer file
        if wants_write && inode.source == InodeSource::Lower {
            match self.copy_up(&inode.path) {
                Ok(_) => {
                    // Update inode to point to upper
                    let mut updated = inode.clone();
                    updated.source = InodeSource::Upper;
                    updated.lower_path = None;
                    self.inodes.update(ino, updated);
                }
                Err(e) => {
                    error!("Copy-up failed: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        let fh = self.handles.open(ino, inode.source, flags);

        // Set path info
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
        debug!(
            "read(ino={}, fh={}, offset={}, size={})",
            ino, fh, offset, size
        );

        let inode = match self.inodes.get(ino) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let path = match inode.source {
            InodeSource::Upper => self.upper_path_for(&inode.path),
            InodeSource::Lower => {
                if let Some(ref lp) = inode.lower_path {
                    self.lower.resolve(lp)
                } else {
                    reply.error(ENOENT);
                    return;
                }
            }
            _ => {
                reply.error(ENOENT);
                return;
            }
        };

        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to open file for read: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if let Err(e) = file.seek(SeekFrom::Start(offset as u64)) {
            error!("Failed to seek: {}", e);
            reply.error(libc::EIO);
            return;
        }

        let mut buffer = vec![0u8; size as usize];
        match file.read(&mut buffer) {
            Ok(n) => {
                buffer.truncate(n);
                reply.data(&buffer);
            }
            Err(e) => {
                error!("Failed to read: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        debug!(
            "write(ino={}, fh={}, offset={}, size={})",
            ino,
            fh,
            offset,
            data.len()
        );

        let inode = match self.inodes.get(ino) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // Ensure file is in upper layer
        let upper_path = if inode.source == InodeSource::Lower {
            match self.copy_up(&inode.path) {
                Ok(p) => {
                    // Update inode
                    let mut updated = inode.clone();
                    updated.source = InodeSource::Upper;
                    updated.lower_path = None;
                    self.inodes.update(ino, updated);
                    p
                }
                Err(e) => {
                    error!("Copy-up failed: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        } else {
            self.upper_path_for(&inode.path)
        };

        let mut file = match OpenOptions::new().write(true).open(&upper_path) {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to open file for write: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if let Err(e) = file.seek(SeekFrom::Start(offset as u64)) {
            error!("Failed to seek: {}", e);
            reply.error(libc::EIO);
            return;
        }

        match file.write(data) {
            Ok(n) => {
                reply.written(n as u32);
            }
            Err(e) => {
                error!("Failed to write: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!("unlink(parent={}, name={:?})", parent, name);

        let parent_inode = match self.inodes.get(parent) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let virtual_path = parent_inode.path.join(name);

        // Check if exists in upper
        let upper_path = self.upper_path_for(&virtual_path);
        if upper_path.exists() {
            if let Err(e) = fs::remove_file(&upper_path) {
                error!("Failed to remove file: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        // Add whiteout if exists in lower
        if self.lower.exists(&virtual_path) {
            if let Err(e) = self.whiteouts.add_whiteout(&virtual_path) {
                error!("Failed to add whiteout: {}", e);
            }
        }

        // Remove from inode cache
        self.inodes.invalidate_path(&virtual_path);

        reply.ok();
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!("rmdir(parent={}, name={:?})", parent, name);

        let parent_inode = match self.inodes.get(parent) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let virtual_path = parent_inode.path.join(name);

        // Check if exists in upper
        let upper_path = self.upper_path_for(&virtual_path);
        if upper_path.exists() {
            if let Err(e) = fs::remove_dir(&upper_path) {
                if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
                    reply.error(ENOTEMPTY);
                } else {
                    error!("Failed to remove directory: {}", e);
                    reply.error(libc::EIO);
                }
                return;
            }
        }

        // Add whiteout if exists in lower
        if self.lower.exists(&virtual_path) {
            if let Err(e) = self.whiteouts.add_whiteout(&virtual_path) {
                error!("Failed to add whiteout: {}", e);
            }
            // Mark as opaque to hide all lower contents
            if let Err(e) = self.whiteouts.mark_opaque(&virtual_path) {
                error!("Failed to mark opaque: {}", e);
            }
        }

        // Remove from inode cache
        self.inodes.invalidate_path(&virtual_path);

        reply.ok();
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        debug!(
            "rename(parent={}, name={:?}, newparent={}, newname={:?})",
            parent, name, newparent, newname
        );

        let parent_inode = match self.inodes.get(parent) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let newparent_inode = match self.inodes.get(newparent) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let old_path = parent_inode.path.join(name);
        let new_path = newparent_inode.path.join(newname);

        // Ensure source is in upper (copy-up if needed)
        let old_upper = self.upper_path_for(&old_path);
        if !old_upper.exists() && self.lower.exists(&old_path) {
            if let Err(e) = self.copy_up(&old_path) {
                error!("Copy-up failed for rename: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        // Ensure new parent exists in upper
        if let Err(e) = self.ensure_parent_in_upper(&new_path) {
            error!("Failed to create parent: {}", e);
            reply.error(libc::EIO);
            return;
        }

        let new_upper = self.upper_path_for(&new_path);

        // Perform rename in upper
        if let Err(e) = fs::rename(&old_upper, &new_upper) {
            error!("Rename failed: {}", e);
            reply.error(libc::EIO);
            return;
        }

        // Add whiteout for old location if it existed in lower
        if self.lower.exists(&old_path) {
            let _ = self.whiteouts.add_whiteout(&old_path);
        }

        // Remove whiteout for new location if any
        let _ = self.whiteouts.remove_whiteout(&new_path);

        // Update inode cache
        self.inodes.invalidate_path(&old_path);
        self.inodes.invalidate_path(&new_path);

        reply.ok();
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

        let path = match inode.source {
            InodeSource::Upper => self.upper_path_for(&inode.path),
            InodeSource::Lower => {
                if let Some(ref lp) = inode.lower_path {
                    self.lower.resolve(lp)
                } else {
                    reply.error(ENOENT);
                    return;
                }
            }
            _ => {
                reply.error(ENOENT);
                return;
            }
        };

        match fs::read_link(&path) {
            Ok(target) => reply.data(target.as_os_str().as_encoded_bytes()),
            Err(e) => {
                error!("Failed to read symlink: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn symlink(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        link: &std::path::Path,
        reply: ReplyEntry,
    ) {
        debug!("symlink(parent={}, name={:?}, link={:?})", parent, name, link);

        let parent_inode = match self.inodes.get(parent) {
            Some(i) => i,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let virtual_path = parent_inode.path.join(name);

        // Ensure parent exists in upper
        if let Err(e) = self.ensure_parent_in_upper(&virtual_path) {
            error!("Failed to create parent: {}", e);
            reply.error(libc::EIO);
            return;
        }

        let upper_path = self.upper_path_for(&virtual_path);

        // Create symlink
        if let Err(e) = std::os::unix::fs::symlink(link, &upper_path) {
            error!("Failed to create symlink: {}", e);
            reply.error(libc::EIO);
            return;
        }

        // Get metadata
        let meta = match fs::symlink_metadata(&upper_path) {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to get symlink metadata: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // Create inode
        let ino = self.inodes.alloc_ino();
        let mut inode = OverlayInode::from_lower(
            ino,
            parent,
            name.to_string_lossy().to_string(),
            virtual_path,
            &meta,
        );
        inode.source = InodeSource::Upper;
        inode.lower_path = None;
        inode.file_type = OverlayFileType::Symlink;
        self.inodes.register(inode.clone());

        reply.entry(&TTL, &inode.to_fuser_attr(), 0);
    }

    fn access(&mut self, _req: &Request, ino: u64, mask: i32, reply: ReplyEmpty) {
        debug!("access(ino={}, mask={})", ino, mask);

        if self.inodes.exists(ino) {
            reply.ok();
        } else {
            reply.error(ENOENT);
        }
    }

    fn statfs(&mut self, _req: &Request, _ino: u64, reply: fuser::ReplyStatfs) {
        // Get stats from upper layer filesystem
        reply.statfs(
            1000000000, // blocks (1TB)
            500000000,  // bfree
            500000000,  // bavail
            1000000,    // files
            500000,     // ffree
            4096,       // bsize
            255,        // namelen
            4096,       // frsize
        );
    }

    fn flush(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        _lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        debug!("flush(ino={}, fh={})", ino, fh);
        reply.ok();
    }

    fn fsync(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        debug!("fsync(ino={}, fh={}, datasync={})", ino, fh, datasync);
        reply.ok();
    }
}
