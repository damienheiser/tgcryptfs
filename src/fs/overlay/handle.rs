//! Overlay file handle management

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::InodeSource;

/// Overlay file handle tracking
pub struct OverlayFileHandle {
    /// Virtual file handle ID
    pub fh: u64,
    /// Virtual inode
    pub ino: u64,
    /// Source at time of open
    pub source: InodeSource,
    /// Upper layer file handle (if opened in upper)
    pub upper_fh: Option<u64>,
    /// Lower layer file path (if opened in lower)
    pub lower_path: Option<std::path::PathBuf>,
    /// Open flags
    pub flags: i32,
    /// Has the file been modified?
    pub dirty: AtomicBool,
    /// Position in file (for sequential reads)
    pub position: AtomicU64,
}

impl OverlayFileHandle {
    /// Create a new file handle
    pub fn new(fh: u64, ino: u64, source: InodeSource, flags: i32) -> Self {
        Self {
            fh,
            ino,
            source,
            upper_fh: None,
            lower_path: None,
            flags,
            dirty: AtomicBool::new(false),
            position: AtomicU64::new(0),
        }
    }

    /// Check if opened for writing
    pub fn is_writable(&self) -> bool {
        let accmode = self.flags & libc::O_ACCMODE;
        accmode == libc::O_WRONLY || accmode == libc::O_RDWR
    }

    /// Check if dirty (modified)
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    /// Mark as dirty
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Update position
    pub fn set_position(&self, pos: u64) {
        self.position.store(pos, Ordering::SeqCst);
    }

    /// Get current position
    pub fn get_position(&self) -> u64 {
        self.position.load(Ordering::SeqCst)
    }
}

/// Manages overlay file handles
pub struct OverlayHandleManager {
    next_fh: AtomicU64,
    handles: RwLock<HashMap<u64, OverlayFileHandle>>,
}

impl OverlayHandleManager {
    pub fn new() -> Self {
        Self {
            next_fh: AtomicU64::new(1),
            handles: RwLock::new(HashMap::new()),
        }
    }

    /// Allocate a new file handle
    pub fn alloc(&self) -> u64 {
        self.next_fh.fetch_add(1, Ordering::SeqCst)
    }

    /// Register a file handle
    pub fn register(&self, handle: OverlayFileHandle) -> u64 {
        let fh = handle.fh;
        self.handles.write().insert(fh, handle);
        fh
    }

    /// Open a file and return handle
    pub fn open(&self, ino: u64, source: InodeSource, flags: i32) -> u64 {
        let fh = self.alloc();
        let handle = OverlayFileHandle::new(fh, ino, source, flags);
        self.register(handle)
    }

    /// Get handle by ID
    pub fn get(&self, fh: u64) -> Option<OverlayFileHandle> {
        self.handles.read().get(&fh).map(|h| OverlayFileHandle {
            fh: h.fh,
            ino: h.ino,
            source: h.source,
            upper_fh: h.upper_fh,
            lower_path: h.lower_path.clone(),
            flags: h.flags,
            dirty: AtomicBool::new(h.dirty.load(Ordering::SeqCst)),
            position: AtomicU64::new(h.position.load(Ordering::SeqCst)),
        })
    }

    /// Update handle's upper file handle
    pub fn set_upper_fh(&self, fh: u64, upper_fh: u64) {
        if let Some(handle) = self.handles.write().get_mut(&fh) {
            handle.upper_fh = Some(upper_fh);
            handle.source = InodeSource::Upper;
        }
    }

    /// Update handle's lower path
    pub fn set_lower_path(&self, fh: u64, path: std::path::PathBuf) {
        if let Some(handle) = self.handles.write().get_mut(&fh) {
            handle.lower_path = Some(path);
        }
    }

    /// Mark handle as dirty
    pub fn mark_dirty(&self, fh: u64) {
        if let Some(handle) = self.handles.read().get(&fh) {
            handle.mark_dirty();
        }
    }

    /// Close handle and return it
    pub fn close(&self, fh: u64) -> Option<OverlayFileHandle> {
        self.handles.write().remove(&fh).map(|h| OverlayFileHandle {
            fh: h.fh,
            ino: h.ino,
            source: h.source,
            upper_fh: h.upper_fh,
            lower_path: h.lower_path.clone(),
            flags: h.flags,
            dirty: AtomicBool::new(h.dirty.load(Ordering::SeqCst)),
            position: AtomicU64::new(h.position.load(Ordering::SeqCst)),
        })
    }

    /// Get all handles for an inode
    pub fn handles_for_inode(&self, ino: u64) -> Vec<u64> {
        self.handles
            .read()
            .values()
            .filter(|h| h.ino == ino)
            .map(|h| h.fh)
            .collect()
    }
}

impl Default for OverlayHandleManager {
    fn default() -> Self {
        Self::new()
    }
}
