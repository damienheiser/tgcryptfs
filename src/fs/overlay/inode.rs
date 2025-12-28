//! Overlay inode management
//!
//! Manages virtual inodes that merge upper and lower layer entries.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Source layer for an inode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeSource {
    /// Inode exists only in upper layer
    Upper,
    /// Inode exists only in lower layer
    Lower,
    /// Inode exists in both layers (merged directory or upper shadows lower)
    Merged,
}

/// File type for overlay inode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayFileType {
    RegularFile,
    Directory,
    Symlink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
}

impl From<std::fs::FileType> for OverlayFileType {
    fn from(ft: std::fs::FileType) -> Self {
        if ft.is_file() {
            OverlayFileType::RegularFile
        } else if ft.is_dir() {
            OverlayFileType::Directory
        } else if ft.is_symlink() {
            OverlayFileType::Symlink
        } else {
            OverlayFileType::RegularFile // Default
        }
    }
}

impl OverlayFileType {
    pub fn to_fuser_type(&self) -> fuser::FileType {
        match self {
            OverlayFileType::RegularFile => fuser::FileType::RegularFile,
            OverlayFileType::Directory => fuser::FileType::Directory,
            OverlayFileType::Symlink => fuser::FileType::Symlink,
            OverlayFileType::BlockDevice => fuser::FileType::BlockDevice,
            OverlayFileType::CharDevice => fuser::FileType::CharDevice,
            OverlayFileType::Fifo => fuser::FileType::NamedPipe,
            OverlayFileType::Socket => fuser::FileType::Socket,
        }
    }
}

/// Attributes for overlay inode
#[derive(Debug, Clone)]
pub struct OverlayAttributes {
    pub size: u64,
    pub blocks: u64,
    pub atime: SystemTime,
    pub mtime: SystemTime,
    pub ctime: SystemTime,
    pub crtime: SystemTime,
    pub perm: u16,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
}

impl Default for OverlayAttributes {
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            perm: 0o644,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
        }
    }
}

impl OverlayAttributes {
    #[cfg(unix)]
    pub fn from_metadata(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            size: meta.len(),
            blocks: meta.blocks(),
            atime: meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH),
            mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ctime: SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(meta.ctime() as u64),
            crtime: meta.created().unwrap_or(SystemTime::UNIX_EPOCH),
            perm: (meta.mode() & 0o7777) as u16,
            nlink: meta.nlink() as u32,
            uid: meta.uid(),
            gid: meta.gid(),
            rdev: meta.rdev() as u32,
            blksize: meta.blksize() as u32,
        }
    }

    #[cfg(not(unix))]
    pub fn from_metadata(meta: &std::fs::Metadata) -> Self {
        Self {
            size: meta.len(),
            blocks: (meta.len() + 511) / 512,
            atime: meta.accessed().unwrap_or(SystemTime::UNIX_EPOCH),
            mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ctime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            crtime: meta.created().unwrap_or(SystemTime::UNIX_EPOCH),
            perm: if meta.is_dir() { 0o755 } else { 0o644 },
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 4096,
        }
    }
}

/// Extended inode for overlay filesystem
#[derive(Debug, Clone)]
pub struct OverlayInode {
    /// Virtual inode number (unique within overlay)
    pub ino: u64,
    /// Parent virtual inode
    pub parent: u64,
    /// File/directory name
    pub name: String,
    /// Virtual path (relative to overlay root)
    pub path: PathBuf,
    /// Source layer
    pub source: InodeSource,
    /// Upper layer inode (if exists in upper)
    pub upper_ino: Option<u64>,
    /// Lower layer path (if exists in lower)
    pub lower_path: Option<PathBuf>,
    /// File type
    pub file_type: OverlayFileType,
    /// Cached attributes (merged from upper or lower)
    pub attrs: OverlayAttributes,
    /// Is this inode copied up?
    pub copied_up: bool,
}

impl OverlayInode {
    /// Create root inode
    pub fn root() -> Self {
        Self {
            ino: 1,
            parent: 1,
            name: String::new(),
            path: PathBuf::from("/"),
            source: InodeSource::Merged, // Root is always merged
            upper_ino: Some(1),
            lower_path: Some(PathBuf::from("/")),
            file_type: OverlayFileType::Directory,
            attrs: OverlayAttributes {
                perm: 0o755,
                nlink: 2,
                ..Default::default()
            },
            copied_up: false,
        }
    }

    /// Create from lower layer metadata
    pub fn from_lower(
        ino: u64,
        parent: u64,
        name: String,
        path: PathBuf,
        meta: &std::fs::Metadata,
    ) -> Self {
        Self {
            ino,
            parent,
            name,
            path: path.clone(),
            source: InodeSource::Lower,
            upper_ino: None,
            lower_path: Some(path),
            file_type: OverlayFileType::from(meta.file_type()),
            attrs: OverlayAttributes::from_metadata(meta),
            copied_up: false,
        }
    }

    /// Convert to fuser FileAttr
    pub fn to_fuser_attr(&self) -> fuser::FileAttr {
        fuser::FileAttr {
            ino: self.ino,
            size: self.attrs.size,
            blocks: self.attrs.blocks,
            atime: self.attrs.atime,
            mtime: self.attrs.mtime,
            ctime: self.attrs.ctime,
            crtime: self.attrs.crtime,
            kind: self.file_type.to_fuser_type(),
            perm: self.attrs.perm,
            nlink: self.attrs.nlink,
            uid: self.attrs.uid,
            gid: self.attrs.gid,
            rdev: self.attrs.rdev,
            blksize: self.attrs.blksize,
            flags: 0,
        }
    }
}

/// Manages virtual inode allocation and mapping
pub struct OverlayInodeManager {
    /// Next virtual inode number
    next_ino: AtomicU64,
    /// Virtual ino -> OverlayInode
    inodes: RwLock<HashMap<u64, OverlayInode>>,
    /// Path -> Virtual ino (for lookups)
    path_to_ino: RwLock<HashMap<PathBuf, u64>>,
}

impl OverlayInodeManager {
    pub fn new() -> Self {
        let manager = Self {
            next_ino: AtomicU64::new(2), // 1 is reserved for root
            inodes: RwLock::new(HashMap::new()),
            path_to_ino: RwLock::new(HashMap::new()),
        };

        // Register root inode
        let root = OverlayInode::root();
        manager.register(root);

        manager
    }

    /// Allocate a new virtual inode number
    pub fn alloc_ino(&self) -> u64 {
        self.next_ino.fetch_add(1, Ordering::SeqCst)
    }

    /// Register an overlay inode
    pub fn register(&self, inode: OverlayInode) -> u64 {
        let ino = inode.ino;
        let path = inode.path.clone();

        self.inodes.write().insert(ino, inode);
        self.path_to_ino.write().insert(path, ino);

        ino
    }

    /// Get inode by virtual ino
    pub fn get(&self, ino: u64) -> Option<OverlayInode> {
        self.inodes.read().get(&ino).cloned()
    }

    /// Get inode by path
    pub fn get_by_path(&self, path: &PathBuf) -> Option<OverlayInode> {
        let ino = self.path_to_ino.read().get(path).copied()?;
        self.get(ino)
    }

    /// Update an inode (e.g., after copy-up)
    pub fn update(&self, ino: u64, inode: OverlayInode) {
        let old_path = self.inodes.read().get(&ino).map(|i| i.path.clone());
        let new_path = inode.path.clone();

        self.inodes.write().insert(ino, inode);

        // Update path mapping if path changed
        if let Some(old) = old_path {
            if old != new_path {
                self.path_to_ino.write().remove(&old);
                self.path_to_ino.write().insert(new_path, ino);
            }
        }
    }

    /// Remove an inode
    pub fn remove(&self, ino: u64) -> Option<OverlayInode> {
        let inode = self.inodes.write().remove(&ino);
        if let Some(ref inode) = inode {
            self.path_to_ino.write().remove(&inode.path);
        }
        inode
    }

    /// Invalidate path mapping (on rename/delete)
    pub fn invalidate_path(&self, path: &PathBuf) {
        if let Some(ino) = self.path_to_ino.write().remove(path) {
            self.inodes.write().remove(&ino);
        }
    }

    /// Check if inode exists
    pub fn exists(&self, ino: u64) -> bool {
        self.inodes.read().contains_key(&ino)
    }

    /// Get all child inodes of a directory
    pub fn children_of(&self, parent_ino: u64) -> Vec<OverlayInode> {
        self.inodes
            .read()
            .values()
            .filter(|i| i.parent == parent_ino && i.ino != parent_ino)
            .cloned()
            .collect()
    }
}

impl Default for OverlayInodeManager {
    fn default() -> Self {
        Self::new()
    }
}
