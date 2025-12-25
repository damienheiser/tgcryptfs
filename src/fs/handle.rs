//! File handle management

use crate::chunk::Chunk;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Represents an open file
pub struct FileHandle {
    /// Inode number
    pub ino: u64,
    /// Open flags
    pub flags: i32,
    /// Write buffer (for buffered writes)
    pub write_buffer: RwLock<Vec<u8>>,
    /// Read position
    pub read_pos: AtomicU64,
    /// Dirty flag (has uncommitted writes)
    pub dirty: std::sync::atomic::AtomicBool,
}

impl FileHandle {
    /// Create a new file handle
    pub fn new(ino: u64, flags: i32) -> Self {
        FileHandle {
            ino,
            flags,
            write_buffer: RwLock::new(Vec::new()),
            read_pos: AtomicU64::new(0),
            dirty: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Check if opened for reading
    pub fn is_readable(&self) -> bool {
        let mode = self.flags & libc::O_ACCMODE;
        mode == libc::O_RDONLY || mode == libc::O_RDWR
    }

    /// Check if opened for writing
    pub fn is_writable(&self) -> bool {
        let mode = self.flags & libc::O_ACCMODE;
        mode == libc::O_WRONLY || mode == libc::O_RDWR
    }

    /// Check if opened for append
    pub fn is_append(&self) -> bool {
        (self.flags & libc::O_APPEND) != 0
    }

    /// Mark as dirty
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Check if dirty
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    /// Clear dirty flag
    pub fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::SeqCst);
    }

    /// Append to write buffer
    pub fn write(&self, data: &[u8]) {
        self.write_buffer.write().extend_from_slice(data);
        self.mark_dirty();
    }

    /// Get write buffer contents
    pub fn get_write_buffer(&self) -> Vec<u8> {
        self.write_buffer.read().clone()
    }

    /// Clear write buffer
    pub fn clear_write_buffer(&self) {
        self.write_buffer.write().clear();
    }
}

/// Manages open file handles
pub struct HandleManager {
    /// Next handle ID
    next_id: AtomicU64,
    /// Open handles
    handles: RwLock<HashMap<u64, FileHandle>>,
}

impl HandleManager {
    /// Create a new handle manager
    pub fn new() -> Self {
        HandleManager {
            next_id: AtomicU64::new(1),
            handles: RwLock::new(HashMap::new()),
        }
    }

    /// Open a file and return a handle ID
    pub fn open(&self, ino: u64, flags: i32) -> u64 {
        let fh = self.next_id.fetch_add(1, Ordering::SeqCst);
        let handle = FileHandle::new(ino, flags);
        self.handles.write().insert(fh, handle);
        fh
    }

    /// Get a handle by ID
    pub fn get(&self, fh: u64) -> Option<std::sync::Arc<FileHandle>> {
        // Note: This is simplified. In production, you'd use Arc for sharing.
        None // Placeholder
    }

    /// Close a handle
    pub fn close(&self, fh: u64) -> Option<FileHandle> {
        self.handles.write().remove(&fh)
    }

    /// Get handle reference for operations
    pub fn with_handle<F, R>(&self, fh: u64, f: F) -> Option<R>
    where
        F: FnOnce(&FileHandle) -> R,
    {
        self.handles.read().get(&fh).map(f)
    }

    /// Get mutable handle reference
    pub fn with_handle_mut<F, R>(&self, fh: u64, f: F) -> Option<R>
    where
        F: FnOnce(&mut FileHandle) -> R,
    {
        self.handles.write().get_mut(&fh).map(f)
    }

    /// Check if a handle is valid
    pub fn is_valid(&self, fh: u64) -> bool {
        self.handles.read().contains_key(&fh)
    }

    /// Get all handles for an inode
    pub fn handles_for_ino(&self, ino: u64) -> Vec<u64> {
        self.handles
            .read()
            .iter()
            .filter(|(_, h)| h.ino == ino)
            .map(|(&fh, _)| fh)
            .collect()
    }
}

impl Default for HandleManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_handle_flags() {
        let read_handle = FileHandle::new(1, libc::O_RDONLY);
        assert!(read_handle.is_readable());
        assert!(!read_handle.is_writable());

        let write_handle = FileHandle::new(1, libc::O_WRONLY);
        assert!(!write_handle.is_readable());
        assert!(write_handle.is_writable());

        let rw_handle = FileHandle::new(1, libc::O_RDWR);
        assert!(rw_handle.is_readable());
        assert!(rw_handle.is_writable());
    }

    #[test]
    fn test_handle_manager() {
        let manager = HandleManager::new();

        let fh1 = manager.open(1, libc::O_RDONLY);
        let fh2 = manager.open(2, libc::O_RDWR);

        assert!(manager.is_valid(fh1));
        assert!(manager.is_valid(fh2));
        assert!(!manager.is_valid(999));

        manager.close(fh1);
        assert!(!manager.is_valid(fh1));
        assert!(manager.is_valid(fh2));
    }

    #[test]
    fn test_write_buffer() {
        let handle = FileHandle::new(1, libc::O_WRONLY);

        assert!(!handle.is_dirty());

        handle.write(b"hello ");
        handle.write(b"world");

        assert!(handle.is_dirty());
        assert_eq!(handle.get_write_buffer(), b"hello world");

        handle.clear_write_buffer();
        handle.clear_dirty();

        assert!(!handle.is_dirty());
        assert!(handle.get_write_buffer().is_empty());
    }
}
