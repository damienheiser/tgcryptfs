//! Main FUSE filesystem implementation

use crate::cache::ChunkCache;
use crate::chunk::{compress_or_original, decompress, ChunkManifest, ChunkRef, Chunker};
use crate::config::Config;
use crate::crypto::{decrypt, encrypt, KeyManager};
use crate::error::{Error, Result};
use crate::fs::handle::HandleManager;
use crate::metadata::{Inode, MetadataStore};
use crate::telegram::TelegramBackend;

use fuser::{
    FileType as FuserFileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request, TimeOrNow,
};
use std::ffi::OsStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::runtime::Runtime;
use tracing::{debug, error};

/// TTL for cached attributes
const TTL: Duration = Duration::from_secs(1);

/// Main tgcryptfs filesystem
pub struct TgCryptFs {
    /// Configuration
    config: Arc<Config>,
    /// Key manager
    keys: Arc<KeyManager>,
    /// Metadata store
    metadata: Arc<MetadataStore>,
    /// Telegram backend
    telegram: Arc<TelegramBackend>,
    /// Local cache
    cache: Arc<ChunkCache>,
    /// Chunker
    chunker: Chunker,
    /// File handle manager
    handles: HandleManager,
    /// Tokio runtime for async operations
    runtime: Runtime,
    /// UID for this process
    uid: u32,
    /// GID for this process
    gid: u32,
}

impl TgCryptFs {
    /// Create a new tgcryptfs instance
    pub fn new(
        config: Config,
        keys: KeyManager,
        metadata: MetadataStore,
        telegram: TelegramBackend,
        cache: ChunkCache,
    ) -> Result<Self> {
        let runtime = Runtime::new().map_err(|e| Error::Internal(e.to_string()))?;

        let chunker = Chunker::new(&config.chunk);

        Ok(TgCryptFs {
            config: Arc::new(config),
            keys: Arc::new(keys),
            metadata: Arc::new(metadata),
            telegram: Arc::new(telegram),
            cache: Arc::new(cache),
            chunker,
            handles: HandleManager::new(),
            runtime,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        })
    }

    /// Helper to run async code from sync FUSE callbacks
    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.runtime.block_on(f)
    }

    /// Read file data at a given offset
    fn read_file_data(&self, inode: &Inode, offset: u64, size: u32) -> Result<Vec<u8>> {
        let manifest = inode
            .manifest
            .as_ref()
            .ok_or_else(|| Error::NotAFile(inode.name.clone()))?;

        if offset >= manifest.total_size {
            return Ok(Vec::new());
        }

        let end = std::cmp::min(offset + size as u64, manifest.total_size);
        let mut result = Vec::with_capacity((end - offset) as usize);

        // Find chunks that overlap with the requested range
        let mut current_offset = 0u64;
        for chunk_ref in &manifest.chunks {
            let chunk_end = current_offset + chunk_ref.original_size;

            if chunk_end > offset && current_offset < end {
                // This chunk overlaps with our range
                let chunk_data = self.get_chunk_data(chunk_ref)?;

                // Calculate the slice of this chunk we need
                let slice_start = if offset > current_offset {
                    (offset - current_offset) as usize
                } else {
                    0
                };
                let slice_end = if end < chunk_end {
                    (end - current_offset) as usize
                } else {
                    chunk_data.len()
                };

                result.extend_from_slice(&chunk_data[slice_start..slice_end]);
            }

            current_offset = chunk_end;
            if current_offset >= end {
                break;
            }
        }

        Ok(result)
    }

    /// Get chunk data (from cache or Telegram)
    fn get_chunk_data(&self, chunk_ref: &ChunkRef) -> Result<Vec<u8>> {
        // Try cache first
        if let Some(data) = self.cache.get(&chunk_ref.id)? {
            return Ok(data);
        }

        // Download from Telegram
        let encrypted_bytes = self.block_on(self.telegram.download_chunk(chunk_ref.message_id))?;

        // Decrypt
        let chunk_key = self.keys.chunk_key(&chunk_ref.id)?;
        let encrypted = crate::crypto::EncryptedData::from_bytes(&encrypted_bytes)?;
        let decrypted = decrypt(chunk_key.key(), &encrypted, &[])?;

        // Decompress if needed
        let data = if chunk_ref.compressed {
            decompress(&decrypted)?
        } else {
            decrypted
        };

        // Cache for later
        self.cache.put(&chunk_ref.id, &data)?;

        Ok(data)
    }

    /// Write file data (simplified - full implementation would handle partial writes)
    fn write_file_data(&self, ino: u64, data: &[u8]) -> Result<()> {
        let mut inode = self.metadata.get_inode_required(ino)?;

        // Create chunks
        let chunks = self.chunker.chunk_data(data);
        let file_hash = self.chunker.file_hash(data);

        // Create new manifest
        let mut manifest = ChunkManifest::new(inode.version + 1);
        manifest.total_size = data.len() as u64;
        manifest.file_hash = file_hash;

        // Upload each chunk
        for chunk in chunks {
            // Compress if beneficial
            let (chunk_data, compressed) =
                compress_or_original(&chunk.data, self.config.chunk.compression_threshold);

            // Encrypt
            let chunk_key = self.keys.chunk_key(&chunk.info.id)?;
            let encrypted = encrypt(chunk_key.key(), &chunk_data, &[])?;

            // Check if chunk already exists (dedup)
            let message_id = if let Some(msg_id) = self.metadata.get_chunk_ref(&chunk.info.id)? {
                // Chunk already exists, just add reference
                self.metadata.save_chunk_ref(&chunk.info.id, msg_id)?;
                msg_id
            } else {
                // Upload new chunk
                let msg_id = self.block_on(self.telegram.upload_chunk(&chunk.info.id, &encrypted.to_bytes()))?;
                self.metadata.save_chunk_ref(&chunk.info.id, msg_id)?;
                msg_id
            };

            // Add to manifest
            manifest.chunks.push(ChunkRef {
                id: chunk.info.id,
                size: encrypted.size() as u64,
                message_id,
                offset: chunk.info.offset,
                original_size: chunk.data.len() as u64,
                compressed,
            });

            // Cache the uncompressed data
            self.cache.put(&manifest.chunks.last().unwrap().id, &chunk.data)?;
        }

        // Update inode
        inode.manifest = Some(manifest);
        inode.set_size(data.len() as u64);
        inode.bump_version();
        self.metadata.save_inode(&inode)?;

        Ok(())
    }

    /// Create a new file
    fn create_file(&self, parent: u64, name: &str, mode: u32) -> Result<Inode> {
        // Check parent exists and is a directory
        let mut parent_inode = self.metadata.get_inode_required(parent)?;
        if !parent_inode.is_dir() {
            return Err(Error::NotADirectory(parent_inode.name.clone()));
        }

        // Check name doesn't already exist
        if self.metadata.lookup(parent, name)?.is_some() {
            return Err(Error::AlreadyExists(name.to_string()));
        }

        // Create new inode
        let ino = self.metadata.alloc_ino();
        let inode = Inode::new_file(ino, parent, name.to_string(), self.uid, self.gid, mode as u16);

        // Save inode and update parent
        self.metadata.save_inode(&inode)?;
        parent_inode.add_child(ino);
        self.metadata.save_inode(&parent_inode)?;

        Ok(inode)
    }

    /// Create a new directory
    fn create_directory(&self, parent: u64, name: &str, mode: u32) -> Result<Inode> {
        let mut parent_inode = self.metadata.get_inode_required(parent)?;
        if !parent_inode.is_dir() {
            return Err(Error::NotADirectory(parent_inode.name.clone()));
        }

        if self.metadata.lookup(parent, name)?.is_some() {
            return Err(Error::AlreadyExists(name.to_string()));
        }

        let ino = self.metadata.alloc_ino();
        let inode = Inode::new_directory(ino, parent, name.to_string(), self.uid, self.gid, mode as u16);

        self.metadata.save_inode(&inode)?;
        parent_inode.add_child(ino);
        parent_inode.attrs.nlink += 1; // For ..
        self.metadata.save_inode(&parent_inode)?;

        Ok(inode)
    }

    /// Remove a file
    fn remove_file(&self, parent: u64, name: &str) -> Result<()> {
        let mut parent_inode = self.metadata.get_inode_required(parent)?;
        let inode = self
            .metadata
            .lookup(parent, name)?
            .ok_or_else(|| Error::PathNotFound(name.to_string()))?;

        if !inode.is_file() && !inode.is_symlink() {
            return Err(Error::NotAFile(name.to_string()));
        }

        // Decrement chunk references and delete orphaned chunks
        if let Some(manifest) = &inode.manifest {
            for chunk in &manifest.chunks {
                if let Some(msg_id) = self.metadata.decrement_chunk_ref(&chunk.id)? {
                    // Chunk is orphaned, delete from Telegram
                    let _ = self.block_on(self.telegram.delete_message(msg_id));
                    let _ = self.cache.remove(&chunk.id);
                }
            }
        }

        // Delete inode
        self.metadata.delete_inode(inode.ino)?;

        // Update parent
        parent_inode.remove_child(inode.ino);
        self.metadata.save_inode(&parent_inode)?;

        Ok(())
    }

    /// Remove a directory
    fn remove_directory(&self, parent: u64, name: &str) -> Result<()> {
        let mut parent_inode = self.metadata.get_inode_required(parent)?;
        let inode = self
            .metadata
            .lookup(parent, name)?
            .ok_or_else(|| Error::PathNotFound(name.to_string()))?;

        if !inode.is_dir() {
            return Err(Error::NotADirectory(name.to_string()));
        }

        if !inode.children.is_empty() {
            return Err(Error::DirectoryNotEmpty(name.to_string()));
        }

        self.metadata.delete_inode(inode.ino)?;
        parent_inode.remove_child(inode.ino);
        parent_inode.attrs.nlink -= 1;
        self.metadata.save_inode(&parent_inode)?;

        Ok(())
    }
}

impl Filesystem for TgCryptFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("lookup: parent={}, name={}", parent, name);

        match self.metadata.lookup(parent, name) {
            Ok(Some(inode)) => {
                reply.entry(&TTL, &inode.attrs.to_fuser(inode.ino), 0);
            }
            Ok(None) => {
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                error!("lookup error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr: ino={}", ino);

        match self.metadata.get_inode(ino) {
            Ok(Some(inode)) => {
                reply.attr(&TTL, &inode.attrs.to_fuser(ino));
            }
            Ok(None) => {
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                error!("getattr error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn setattr(
        &mut self,
        _req: &Request,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        debug!("setattr: ino={}", ino);

        match self.metadata.get_inode(ino) {
            Ok(Some(mut inode)) => {
                if let Some(m) = mode {
                    inode.attrs.perm = m as u16;
                }
                if let Some(u) = uid {
                    inode.attrs.uid = u;
                }
                if let Some(g) = gid {
                    inode.attrs.gid = g;
                }
                if let Some(s) = size {
                    // Truncate
                    if s == 0 && inode.is_file() {
                        inode.manifest = Some(ChunkManifest::new(inode.version + 1));
                    }
                    inode.set_size(s);
                }
                if let Some(a) = atime {
                    inode.attrs.atime = match a {
                        TimeOrNow::SpecificTime(t) => t,
                        TimeOrNow::Now => SystemTime::now(),
                    };
                }
                if let Some(m) = mtime {
                    inode.attrs.mtime = match m {
                        TimeOrNow::SpecificTime(t) => t,
                        TimeOrNow::Now => SystemTime::now(),
                    };
                }

                inode.attrs.ctime = SystemTime::now();

                match self.metadata.save_inode(&inode) {
                    Ok(_) => reply.attr(&TTL, &inode.attrs.to_fuser(ino)),
                    Err(e) => reply.error(e.to_errno()),
                }
            }
            Ok(None) => reply.error(libc::ENOENT),
            Err(e) => reply.error(e.to_errno()),
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
        debug!("readdir: ino={}, offset={}", ino, offset);

        let inode = match self.metadata.get_inode(ino) {
            Ok(Some(i)) => i,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                reply.error(e.to_errno());
                return;
            }
        };

        if !inode.is_dir() {
            reply.error(libc::ENOTDIR);
            return;
        }

        let mut entries = vec![
            (ino, FuserFileType::Directory, ".".to_string()),
            (inode.parent, FuserFileType::Directory, "..".to_string()),
        ];

        // Get children
        match self.metadata.get_children(ino) {
            Ok(children) => {
                for child in children {
                    entries.push((child.ino, child.attrs.kind.to_fuser(), child.name.clone()));
                }
            }
            Err(e) => {
                reply.error(e.to_errno());
                return;
            }
        }

        for (i, (ino, kind, name)) in entries.iter().enumerate().skip(offset as usize) {
            if reply.add(*ino, (i + 1) as i64, *kind, name) {
                break;
            }
        }

        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("open: ino={}, flags={}", ino, flags);

        match self.metadata.get_inode(ino) {
            Ok(Some(inode)) => {
                if inode.is_dir() {
                    reply.error(libc::EISDIR);
                    return;
                }
                let fh = self.handles.open(ino, flags);
                reply.opened(fh, 0);
            }
            Ok(None) => reply.error(libc::ENOENT),
            Err(e) => reply.error(e.to_errno()),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read: ino={}, offset={}, size={}", ino, offset, size);

        let inode = match self.metadata.get_inode(ino) {
            Ok(Some(i)) => i,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                reply.error(e.to_errno());
                return;
            }
        };

        match self.read_file_data(&inode, offset as u64, size) {
            Ok(data) => reply.data(&data),
            Err(e) => {
                error!("read error: {}", e);
                reply.error(e.to_errno());
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
        debug!("write: ino={}, offset={}, size={}", ino, offset, data.len());

        // For simplicity, we buffer writes and flush on release
        // A full implementation would handle partial writes and offsets
        self.handles.with_handle_mut(fh, |handle| {
            handle.write(data);
        });

        reply.written(data.len() as u32);
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
        debug!("release: ino={}, fh={}", ino, fh);

        if let Some(handle) = self.handles.close(fh) {
            if handle.is_dirty() {
                let data = handle.get_write_buffer();
                if let Err(e) = self.write_file_data(ino, &data) {
                    error!("Failed to flush write buffer: {}", e);
                    reply.error(e.to_errno());
                    return;
                }
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
        let name = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("create: parent={}, name={}, mode={:o}", parent, name, mode);

        match self.create_file(parent, name, mode) {
            Ok(inode) => {
                let fh = self.handles.open(inode.ino, flags);
                reply.created(&TTL, &inode.attrs.to_fuser(inode.ino), 0, fh, 0);
            }
            Err(e) => {
                error!("create error: {}", e);
                reply.error(e.to_errno());
            }
        }
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
        let name = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("mkdir: parent={}, name={}, mode={:o}", parent, name, mode);

        match self.create_directory(parent, name, mode) {
            Ok(inode) => {
                reply.entry(&TTL, &inode.attrs.to_fuser(inode.ino), 0);
            }
            Err(e) => {
                error!("mkdir error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("unlink: parent={}, name={}", parent, name);

        match self.remove_file(parent, name) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!("unlink error: {}", e);
                reply.error(e.to_errno());
            }
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("rmdir: parent={}, name={}", parent, name);

        match self.remove_directory(parent, name) {
            Ok(_) => reply.ok(),
            Err(e) => {
                error!("rmdir error: {}", e);
                reply.error(e.to_errno());
            }
        }
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
        let name = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        let newname = match newname.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!(
            "rename: parent={}, name={}, newparent={}, newname={}",
            parent, name, newparent, newname
        );

        // Get source inode
        let mut inode = match self.metadata.lookup(parent, name) {
            Ok(Some(i)) => i,
            Ok(None) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                reply.error(e.to_errno());
                return;
            }
        };

        // Check if target exists
        if let Ok(Some(existing)) = self.metadata.lookup(newparent, newname) {
            // Remove existing
            if existing.is_dir() {
                if let Err(e) = self.remove_directory(newparent, newname) {
                    reply.error(e.to_errno());
                    return;
                }
            } else if let Err(e) = self.remove_file(newparent, newname) {
                reply.error(e.to_errno());
                return;
            }
        }

        // Update old parent
        if let Ok(mut old_parent) = self.metadata.get_inode_required(parent) {
            old_parent.remove_child(inode.ino);
            if inode.is_dir() {
                old_parent.attrs.nlink -= 1;
            }
            let _ = self.metadata.save_inode(&old_parent);
        }

        // Update new parent
        if let Ok(mut new_parent) = self.metadata.get_inode_required(newparent) {
            new_parent.add_child(inode.ino);
            if inode.is_dir() {
                new_parent.attrs.nlink += 1;
            }
            let _ = self.metadata.save_inode(&new_parent);
        }

        // Update inode
        inode.parent = newparent;
        inode.name = newname.to_string();
        inode.attrs.ctime = SystemTime::now();

        match self.metadata.save_inode(&inode) {
            Ok(_) => reply.ok(),
            Err(e) => reply.error(e.to_errno()),
        }
    }

    fn statfs(&mut self, _req: &Request, _ino: u64, reply: fuser::ReplyStatfs) {
        // Return some reasonable values
        // In a full implementation, you'd track actual usage
        reply.statfs(
            1_000_000,   // blocks
            500_000,     // bfree
            500_000,     // bavail
            1_000_000,   // files
            500_000,     // ffree
            4096,        // bsize
            255,         // namelen
            4096,        // frsize
        );
    }
}
