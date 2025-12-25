//! Inode representation for the filesystem
//!
//! Each file and directory is represented by an inode with
//! associated attributes and chunk references.

use crate::chunk::ChunkManifest;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// File type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    /// Regular file
    RegularFile,
    /// Directory
    Directory,
    /// Symbolic link
    Symlink,
}

impl FileType {
    /// Convert to fuser file type
    pub fn to_fuser(&self) -> fuser::FileType {
        match self {
            FileType::RegularFile => fuser::FileType::RegularFile,
            FileType::Directory => fuser::FileType::Directory,
            FileType::Symlink => fuser::FileType::Symlink,
        }
    }
}

/// Inode attributes (POSIX-like)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InodeAttributes {
    /// File size in bytes
    pub size: u64,
    /// Number of blocks
    pub blocks: u64,
    /// Access time
    pub atime: SystemTime,
    /// Modification time
    pub mtime: SystemTime,
    /// Change time
    pub ctime: SystemTime,
    /// Creation time
    pub crtime: SystemTime,
    /// File type
    pub kind: FileType,
    /// Permission mode
    pub perm: u16,
    /// Number of hard links
    pub nlink: u32,
    /// User ID
    pub uid: u32,
    /// Group ID
    pub gid: u32,
    /// Device ID (for special files)
    pub rdev: u32,
    /// Block size
    pub blksize: u32,
    /// Flags
    pub flags: u32,
}

impl InodeAttributes {
    /// Create attributes for a new file
    pub fn new_file(uid: u32, gid: u32, perm: u16) -> Self {
        let now = SystemTime::now();
        InodeAttributes {
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::RegularFile,
            perm,
            nlink: 1,
            uid,
            gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// Create attributes for a new directory
    pub fn new_directory(uid: u32, gid: u32, perm: u16) -> Self {
        let now = SystemTime::now();
        InodeAttributes {
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Directory,
            perm,
            nlink: 2, // . and parent
            uid,
            gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// Create attributes for a symlink
    pub fn new_symlink(uid: u32, gid: u32, target_len: u64) -> Self {
        let now = SystemTime::now();
        InodeAttributes {
            size: target_len,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            crtime: now,
            kind: FileType::Symlink,
            perm: 0o777,
            nlink: 1,
            uid,
            gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// Update modification time
    pub fn touch(&mut self) {
        let now = SystemTime::now();
        self.mtime = now;
        self.ctime = now;
    }

    /// Convert to fuser FileAttr
    pub fn to_fuser(&self, ino: u64) -> fuser::FileAttr {
        fuser::FileAttr {
            ino,
            size: self.size,
            blocks: self.blocks,
            atime: self.atime,
            mtime: self.mtime,
            ctime: self.ctime,
            crtime: self.crtime,
            kind: self.kind.to_fuser(),
            perm: self.perm,
            nlink: self.nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: self.rdev,
            blksize: self.blksize,
            flags: self.flags,
        }
    }
}

/// Inode representing a file or directory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inode {
    /// Inode number
    pub ino: u64,
    /// Parent inode number (0 for root)
    pub parent: u64,
    /// File/directory name
    pub name: String,
    /// Inode attributes
    pub attrs: InodeAttributes,
    /// Chunk manifest (for regular files)
    pub manifest: Option<ChunkManifest>,
    /// Symlink target (for symlinks)
    pub symlink_target: Option<String>,
    /// Child inode numbers (for directories)
    pub children: Vec<u64>,
    /// Current version number
    pub version: u64,
    /// Extended attributes
    pub xattrs: std::collections::HashMap<String, Vec<u8>>,
}

impl Inode {
    /// Create a new root inode
    pub fn root(uid: u32, gid: u32, perm: u16) -> Self {
        Inode {
            ino: 1, // Root is always inode 1
            parent: 1, // Root's parent is itself
            name: String::new(),
            attrs: InodeAttributes::new_directory(uid, gid, perm),
            manifest: None,
            symlink_target: None,
            children: Vec::new(),
            version: 0,
            xattrs: std::collections::HashMap::new(),
        }
    }

    /// Create a new file inode
    pub fn new_file(ino: u64, parent: u64, name: String, uid: u32, gid: u32, perm: u16) -> Self {
        Inode {
            ino,
            parent,
            name,
            attrs: InodeAttributes::new_file(uid, gid, perm),
            manifest: Some(ChunkManifest::new(0)),
            symlink_target: None,
            children: Vec::new(),
            version: 0,
            xattrs: std::collections::HashMap::new(),
        }
    }

    /// Create a new directory inode
    pub fn new_directory(
        ino: u64,
        parent: u64,
        name: String,
        uid: u32,
        gid: u32,
        perm: u16,
    ) -> Self {
        Inode {
            ino,
            parent,
            name,
            attrs: InodeAttributes::new_directory(uid, gid, perm),
            manifest: None,
            symlink_target: None,
            children: Vec::new(),
            version: 0,
            xattrs: std::collections::HashMap::new(),
        }
    }

    /// Create a new symlink inode
    pub fn new_symlink(
        ino: u64,
        parent: u64,
        name: String,
        target: String,
        uid: u32,
        gid: u32,
    ) -> Self {
        let target_len = target.len() as u64;
        Inode {
            ino,
            parent,
            name,
            attrs: InodeAttributes::new_symlink(uid, gid, target_len),
            manifest: None,
            symlink_target: Some(target),
            children: Vec::new(),
            version: 0,
            xattrs: std::collections::HashMap::new(),
        }
    }

    /// Check if this is a directory
    pub fn is_dir(&self) -> bool {
        self.attrs.kind == FileType::Directory
    }

    /// Check if this is a regular file
    pub fn is_file(&self) -> bool {
        self.attrs.kind == FileType::RegularFile
    }

    /// Check if this is a symlink
    pub fn is_symlink(&self) -> bool {
        self.attrs.kind == FileType::Symlink
    }

    /// Add a child to a directory
    pub fn add_child(&mut self, child_ino: u64) {
        if !self.children.contains(&child_ino) {
            self.children.push(child_ino);
            self.attrs.touch();
        }
    }

    /// Remove a child from a directory
    pub fn remove_child(&mut self, child_ino: u64) {
        self.children.retain(|&c| c != child_ino);
        self.attrs.touch();
    }

    /// Update file size
    pub fn set_size(&mut self, size: u64) {
        self.attrs.size = size;
        self.attrs.blocks = (size + 511) / 512;
        self.attrs.touch();
    }

    /// Increment version
    pub fn bump_version(&mut self) {
        self.version += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_inode() {
        let root = Inode::root(1000, 1000, 0o755);
        assert_eq!(root.ino, 1);
        assert!(root.is_dir());
        assert!(root.children.is_empty());
    }

    #[test]
    fn test_file_inode() {
        let file = Inode::new_file(2, 1, "test.txt".to_string(), 1000, 1000, 0o644);
        assert!(file.is_file());
        assert!(!file.is_dir());
        assert!(file.manifest.is_some());
    }

    #[test]
    fn test_directory_children() {
        let mut dir = Inode::new_directory(2, 1, "subdir".to_string(), 1000, 1000, 0o755);
        assert!(dir.children.is_empty());

        dir.add_child(3);
        dir.add_child(4);
        assert_eq!(dir.children.len(), 2);

        dir.remove_child(3);
        assert_eq!(dir.children.len(), 1);
        assert!(dir.children.contains(&4));
    }

    #[test]
    fn test_symlink() {
        let link = Inode::new_symlink(
            3,
            1,
            "link".to_string(),
            "/path/to/target".to_string(),
            1000,
            1000,
        );
        assert!(link.is_symlink());
        assert_eq!(link.symlink_target, Some("/path/to/target".to_string()));
    }
}
