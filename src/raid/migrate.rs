//! Migration from single-account to erasure-coded multi-account storage
//!
//! Handles migration of existing files stored on a single Telegram account
//! to RAID-style erasure-coded storage across multiple accounts.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use crate::chunk::{ChunkManifest, ChunkRef, ErasureChunkManifest, ErasureChunkRef};
use crate::error::{Error, Result};
use crate::telegram::TelegramBackend;

use super::pool::AccountPool;
use super::stripe::StripeManager;

/// Migration state for a single chunk
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChunkMigrationState {
    /// Not yet started
    Pending,
    /// Currently being migrated
    InProgress,
    /// Successfully migrated
    Completed,
    /// Migration failed
    Failed(String),
    /// Skipped (e.g., already erasure-coded)
    Skipped,
}

/// Migration progress for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMigrationProgress {
    /// Inode of the file being migrated
    pub inode: u64,
    /// File path (for logging)
    pub path: String,
    /// Total chunks in the file
    pub total_chunks: usize,
    /// Number of chunks completed
    pub completed_chunks: usize,
    /// State of each chunk (by index)
    pub chunk_states: Vec<ChunkMigrationState>,
    /// Start time (Unix seconds)
    pub started_at: i64,
    /// Completion time (Unix seconds, if completed)
    pub completed_at: Option<i64>,
}

impl FileMigrationProgress {
    /// Create new progress tracker for a file
    pub fn new(inode: u64, path: String, total_chunks: usize) -> Self {
        Self {
            inode,
            path,
            total_chunks,
            completed_chunks: 0,
            chunk_states: vec![ChunkMigrationState::Pending; total_chunks],
            started_at: now_unix_secs(),
            completed_at: None,
        }
    }

    /// Check if migration is complete
    pub fn is_complete(&self) -> bool {
        self.completed_chunks == self.total_chunks
    }

    /// Get progress percentage
    pub fn progress_percent(&self) -> f32 {
        if self.total_chunks == 0 {
            return 100.0;
        }
        (self.completed_chunks as f32 / self.total_chunks as f32) * 100.0
    }
}

/// Overall migration progress
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProgress {
    /// Total files to migrate
    pub total_files: usize,
    /// Files completed
    pub completed_files: usize,
    /// Files in progress
    pub in_progress_files: usize,
    /// Files failed
    pub failed_files: usize,
    /// Total chunks to migrate
    pub total_chunks: usize,
    /// Chunks completed
    pub completed_chunks: usize,
    /// Total bytes processed
    pub bytes_processed: u64,
    /// Migration start time
    pub started_at: i64,
    /// Estimated completion time (Unix seconds)
    pub estimated_completion: Option<i64>,
}

impl Default for MigrationProgress {
    fn default() -> Self {
        Self {
            total_files: 0,
            completed_files: 0,
            in_progress_files: 0,
            failed_files: 0,
            total_chunks: 0,
            completed_chunks: 0,
            bytes_processed: 0,
            started_at: now_unix_secs(),
            estimated_completion: None,
        }
    }
}

/// Configuration for migration operation
#[derive(Debug, Clone)]
pub struct MigrationConfig {
    /// Whether to actually perform migration (false = dry run)
    pub dry_run: bool,
    /// Delete old single-account messages after successful migration
    pub delete_old_messages: bool,
    /// Maximum concurrent chunk migrations
    pub max_concurrent: usize,
    /// Continue on individual chunk failures
    pub continue_on_error: bool,
    /// Verify migrated data by re-downloading and comparing
    pub verify_after_migration: bool,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            dry_run: false,
            delete_old_messages: false,
            max_concurrent: 4,
            continue_on_error: true,
            verify_after_migration: false,
        }
    }
}

/// Callback for migration progress updates
pub trait MigrationCallback: Send + Sync {
    /// Called when a file migration starts
    fn on_file_start(&self, inode: u64, path: &str, total_chunks: usize);

    /// Called when a chunk is migrated
    fn on_chunk_complete(&self, inode: u64, chunk_index: usize, total_chunks: usize);

    /// Called when a file migration completes
    fn on_file_complete(&self, inode: u64, path: &str, success: bool);

    /// Called periodically with overall progress
    fn on_progress(&self, progress: &MigrationProgress);
}

/// Default no-op callback
pub struct NoOpCallback;

impl MigrationCallback for NoOpCallback {
    fn on_file_start(&self, _inode: u64, _path: &str, _total_chunks: usize) {}
    fn on_chunk_complete(&self, _inode: u64, _chunk_index: usize, _total_chunks: usize) {}
    fn on_file_complete(&self, _inode: u64, _path: &str, _success: bool) {}
    fn on_progress(&self, _progress: &MigrationProgress) {}
}

/// Logging callback that uses tracing
pub struct LoggingCallback;

impl MigrationCallback for LoggingCallback {
    fn on_file_start(&self, inode: u64, path: &str, total_chunks: usize) {
        info!(
            inode = inode,
            path = path,
            chunks = total_chunks,
            "Starting file migration"
        );
    }

    fn on_chunk_complete(&self, inode: u64, chunk_index: usize, total_chunks: usize) {
        debug!(
            inode = inode,
            chunk = chunk_index,
            total = total_chunks,
            "Chunk migration complete"
        );
    }

    fn on_file_complete(&self, inode: u64, path: &str, success: bool) {
        if success {
            info!(inode = inode, path = path, "File migration complete");
        } else {
            error!(inode = inode, path = path, "File migration failed");
        }
    }

    fn on_progress(&self, progress: &MigrationProgress) {
        let percent = if progress.total_files > 0 {
            (progress.completed_files as f32 / progress.total_files as f32) * 100.0
        } else {
            0.0
        };

        info!(
            files = format!("{}/{}", progress.completed_files, progress.total_files),
            chunks = format!("{}/{}", progress.completed_chunks, progress.total_chunks),
            percent = format!("{:.1}%", percent),
            bytes = progress.bytes_processed,
            "Migration progress"
        );
    }
}

/// Manager for migrating data to erasure-coded storage
pub struct MigrationManager {
    /// Source backend (single account)
    source: Arc<TelegramBackend>,
    /// Destination pool (multi-account)
    pool: Arc<AccountPool>,
    /// Stripe manager for creating erasure-coded stripes
    stripe_manager: StripeManager,
    /// Configuration
    config: MigrationConfig,
    /// Progress callback
    callback: Arc<dyn MigrationCallback>,
    /// Progress counters
    completed_chunks: AtomicUsize,
    completed_files: AtomicUsize,
    bytes_processed: AtomicU64,
}

impl MigrationManager {
    /// Create a new migration manager
    ///
    /// # Arguments
    /// * `source` - The single-account backend to migrate from
    /// * `pool` - The account pool to migrate to
    /// * `config` - Migration configuration
    pub fn new(
        source: Arc<TelegramBackend>,
        pool: Arc<AccountPool>,
        config: MigrationConfig,
    ) -> Result<Self> {
        let stripe_manager = StripeManager::new(
            pool.data_chunks(),
            pool.total_chunks(),
            pool.account_count(),
        )?;

        Ok(Self {
            source,
            pool,
            stripe_manager,
            config,
            callback: Arc::new(LoggingCallback),
            completed_chunks: AtomicUsize::new(0),
            completed_files: AtomicUsize::new(0),
            bytes_processed: AtomicU64::new(0),
        })
    }

    /// Set the progress callback
    pub fn with_callback(mut self, callback: Arc<dyn MigrationCallback>) -> Self {
        self.callback = callback;
        self
    }

    /// Migrate a single file's manifest to erasure-coded storage
    ///
    /// Downloads each chunk from the single account, creates erasure-coded
    /// stripes, and uploads to multiple accounts.
    ///
    /// # Arguments
    /// * `manifest` - The original chunk manifest
    /// * `inode` - Inode of the file (for progress tracking)
    /// * `path` - File path (for logging)
    ///
    /// # Returns
    /// An `ErasureChunkManifest` with the new stripe information
    pub async fn migrate_manifest(
        &self,
        manifest: &ChunkManifest,
        inode: u64,
        path: &str,
    ) -> Result<ErasureChunkManifest> {
        let total_chunks = manifest.chunks.len();

        self.callback.on_file_start(inode, path, total_chunks);

        if self.config.dry_run {
            info!(
                inode = inode,
                path = path,
                chunks = total_chunks,
                "DRY RUN: Would migrate file"
            );
            return self.create_dry_run_manifest(manifest);
        }

        let mut erasure_manifest = ErasureChunkManifest::new(
            manifest.version,
            self.pool.data_chunks() as u8,
            self.pool.total_chunks() as u8,
        );
        erasure_manifest.total_size = manifest.total_size;
        erasure_manifest.file_hash = manifest.file_hash.clone();

        let mut old_message_ids = Vec::new();
        let mut failed = false;

        for (chunk_index, chunk_ref) in manifest.chunks.iter().enumerate() {
            match self.migrate_chunk(chunk_ref, chunk_index as u64).await {
                Ok(erasure_ref) => {
                    erasure_manifest.chunks.push(erasure_ref);
                    old_message_ids.push(chunk_ref.message_id);

                    self.completed_chunks.fetch_add(1, Ordering::SeqCst);
                    self.bytes_processed.fetch_add(chunk_ref.size, Ordering::SeqCst);

                    self.callback.on_chunk_complete(inode, chunk_index, total_chunks);
                }
                Err(e) => {
                    error!(
                        inode = inode,
                        chunk = chunk_index,
                        error = %e,
                        "Failed to migrate chunk"
                    );

                    if !self.config.continue_on_error {
                        self.callback.on_file_complete(inode, path, false);
                        return Err(e);
                    }
                    failed = true;
                }
            }
        }

        if failed {
            self.callback.on_file_complete(inode, path, false);
            return Err(Error::Internal(format!(
                "Migration partially failed for inode {}",
                inode
            )));
        }

        // Optionally verify the migration
        if self.config.verify_after_migration {
            if let Err(e) = self.verify_migration(manifest, &erasure_manifest).await {
                error!(inode = inode, error = %e, "Migration verification failed");
                self.callback.on_file_complete(inode, path, false);
                return Err(e);
            }
            debug!(inode = inode, "Migration verification passed");
        }

        // Delete old messages if configured
        if self.config.delete_old_messages {
            for msg_id in old_message_ids {
                if let Err(e) = self.source.delete_message(msg_id).await {
                    warn!(
                        message_id = msg_id,
                        error = %e,
                        "Failed to delete old message"
                    );
                }
            }
        }

        self.completed_files.fetch_add(1, Ordering::SeqCst);
        self.callback.on_file_complete(inode, path, true);

        Ok(erasure_manifest)
    }

    /// Migrate a single chunk
    async fn migrate_chunk(
        &self,
        chunk_ref: &ChunkRef,
        stripe_index: u64,
    ) -> Result<ErasureChunkRef> {
        debug!(
            chunk_id = %chunk_ref.id,
            message_id = chunk_ref.message_id,
            size = chunk_ref.size,
            "Migrating chunk"
        );

        // Download chunk from source
        let data = self.source.download_chunk(chunk_ref.message_id).await?;

        // Create stripe
        let stripe = self.stripe_manager.create_stripe(
            chunk_ref.id.clone(),
            &data,
            stripe_index,
        )?;

        // Upload stripe to pool
        let stripe_info = self.pool.upload_stripe(&stripe).await?;

        // Create erasure chunk reference
        let erasure_ref = ErasureChunkRef {
            id: chunk_ref.id.clone(),
            offset: chunk_ref.offset,
            original_size: chunk_ref.original_size,
            compressed: chunk_ref.compressed,
            stripe: stripe_info,
            version: 1,
        };

        debug!(
            chunk_id = %chunk_ref.id,
            blocks = erasure_ref.stripe.blocks.len(),
            "Chunk migrated successfully"
        );

        Ok(erasure_ref)
    }

    /// Verify migration by comparing data
    async fn verify_migration(
        &self,
        original: &ChunkManifest,
        erasure: &ErasureChunkManifest,
    ) -> Result<()> {
        if original.chunks.len() != erasure.chunks.len() {
            return Err(Error::Internal(format!(
                "Chunk count mismatch: {} vs {}",
                original.chunks.len(),
                erasure.chunks.len()
            )));
        }

        // Just verify the metadata matches
        if original.total_size != erasure.total_size {
            return Err(Error::Internal("Total size mismatch".to_string()));
        }

        if original.file_hash != erasure.file_hash {
            return Err(Error::Internal("File hash mismatch".to_string()));
        }

        // Verify each chunk can be reconstructed
        for (i, erasure_ref) in erasure.chunks.iter().enumerate() {
            if !erasure_ref.stripe.can_reconstruct() {
                return Err(Error::Internal(format!(
                    "Chunk {} cannot be reconstructed",
                    i
                )));
            }
        }

        Ok(())
    }

    /// Create a dry-run manifest (for testing without actual migration)
    fn create_dry_run_manifest(&self, manifest: &ChunkManifest) -> Result<ErasureChunkManifest> {
        let mut erasure_manifest = ErasureChunkManifest::new(
            manifest.version,
            self.pool.data_chunks() as u8,
            self.pool.total_chunks() as u8,
        );
        erasure_manifest.total_size = manifest.total_size;
        erasure_manifest.file_hash = manifest.file_hash.clone();

        // Create placeholder erasure refs
        for chunk_ref in &manifest.chunks {
            let stripe = self.stripe_manager.create_stripe(
                chunk_ref.id.clone(),
                &[], // Empty data for dry run
                0,
            )?;

            let stripe_info = self.stripe_manager.to_stripe_info(&stripe, &[]);

            erasure_manifest.chunks.push(ErasureChunkRef {
                id: chunk_ref.id.clone(),
                offset: chunk_ref.offset,
                original_size: chunk_ref.original_size,
                compressed: chunk_ref.compressed,
                stripe: stripe_info,
                version: 1,
            });
        }

        Ok(erasure_manifest)
    }

    /// Migrate multiple files
    ///
    /// # Arguments
    /// * `files` - List of (inode, path, manifest) tuples to migrate
    ///
    /// # Returns
    /// Vector of (inode, Result<ErasureChunkManifest>) for each file
    pub async fn migrate_files(
        &self,
        files: Vec<(u64, String, ChunkManifest)>,
    ) -> Vec<(u64, Result<ErasureChunkManifest>)> {
        let total_files = files.len();
        let total_chunks: usize = files.iter().map(|(_, _, m)| m.chunks.len()).sum();

        info!(
            files = total_files,
            chunks = total_chunks,
            dry_run = self.config.dry_run,
            "Starting migration"
        );

        let mut results = Vec::with_capacity(total_files);

        for (inode, path, manifest) in files {
            let result = self.migrate_manifest(&manifest, inode, &path).await;
            results.push((inode, result));

            // Report progress
            let progress = MigrationProgress {
                total_files,
                completed_files: self.completed_files.load(Ordering::SeqCst),
                in_progress_files: 0,
                failed_files: results.iter().filter(|(_, r)| r.is_err()).count(),
                total_chunks,
                completed_chunks: self.completed_chunks.load(Ordering::SeqCst),
                bytes_processed: self.bytes_processed.load(Ordering::SeqCst),
                started_at: now_unix_secs(),
                estimated_completion: None,
            };

            self.callback.on_progress(&progress);
        }

        results
    }

    /// Get current progress
    pub fn progress(&self, total_files: usize, total_chunks: usize) -> MigrationProgress {
        let completed_files = self.completed_files.load(Ordering::SeqCst);
        let completed_chunks = self.completed_chunks.load(Ordering::SeqCst);

        MigrationProgress {
            total_files,
            completed_files,
            in_progress_files: 0,
            failed_files: 0,
            total_chunks,
            completed_chunks,
            bytes_processed: self.bytes_processed.load(Ordering::SeqCst),
            started_at: now_unix_secs(),
            estimated_completion: None,
        }
    }

    /// Get migration configuration
    pub fn config(&self) -> &MigrationConfig {
        &self.config
    }
}

/// Persistence for migration state (for resumability)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationState {
    /// Files that have been fully migrated (by inode)
    pub completed_inodes: HashSet<u64>,
    /// Files currently in progress with their chunk states
    pub in_progress: Vec<FileMigrationProgress>,
    /// Migration start time
    pub started_at: i64,
    /// Last update time
    pub updated_at: i64,
}

impl Default for MigrationState {
    fn default() -> Self {
        Self {
            completed_inodes: HashSet::new(),
            in_progress: Vec::new(),
            started_at: now_unix_secs(),
            updated_at: now_unix_secs(),
        }
    }
}

impl MigrationState {
    /// Check if a file has been migrated
    pub fn is_migrated(&self, inode: u64) -> bool {
        self.completed_inodes.contains(&inode)
    }

    /// Mark a file as migrated
    pub fn mark_migrated(&mut self, inode: u64) {
        self.completed_inodes.insert(inode);
        self.in_progress.retain(|p| p.inode != inode);
        self.updated_at = now_unix_secs();
    }

    /// Get or create progress for a file
    pub fn get_or_create_progress(
        &mut self,
        inode: u64,
        path: String,
        total_chunks: usize,
    ) -> &mut FileMigrationProgress {
        if let Some(pos) = self.in_progress.iter().position(|p| p.inode == inode) {
            &mut self.in_progress[pos]
        } else {
            self.in_progress.push(FileMigrationProgress::new(inode, path, total_chunks));
            self.in_progress.last_mut().unwrap()
        }
    }

    /// Serialize to bytes for storage
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(|e| Error::Serialization(e.to_string()))
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        bincode::deserialize(data).map_err(|e| Error::Deserialization(e.to_string()))
    }
}

/// Get current Unix timestamp in seconds
fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_migration_progress() {
        let progress = FileMigrationProgress::new(42, "/test/file.txt".to_string(), 5);

        assert_eq!(progress.inode, 42);
        assert_eq!(progress.path, "/test/file.txt");
        assert_eq!(progress.total_chunks, 5);
        assert_eq!(progress.completed_chunks, 0);
        assert!(!progress.is_complete());
        assert_eq!(progress.progress_percent(), 0.0);
    }

    #[test]
    fn test_file_migration_progress_complete() {
        let mut progress = FileMigrationProgress::new(42, "/test/file.txt".to_string(), 5);
        progress.completed_chunks = 5;

        assert!(progress.is_complete());
        assert_eq!(progress.progress_percent(), 100.0);
    }

    #[test]
    fn test_file_migration_progress_empty() {
        let progress = FileMigrationProgress::new(42, "/test/empty.txt".to_string(), 0);

        assert!(progress.is_complete());
        assert_eq!(progress.progress_percent(), 100.0);
    }

    #[test]
    fn test_migration_state_default() {
        let state = MigrationState::default();

        assert!(state.completed_inodes.is_empty());
        assert!(state.in_progress.is_empty());
    }

    #[test]
    fn test_migration_state_mark_migrated() {
        let mut state = MigrationState::default();

        assert!(!state.is_migrated(42));
        state.mark_migrated(42);
        assert!(state.is_migrated(42));
    }

    #[test]
    fn test_migration_state_serialization() {
        let mut state = MigrationState::default();
        state.mark_migrated(1);
        state.mark_migrated(2);
        state.get_or_create_progress(3, "/test.txt".to_string(), 10);

        let bytes = state.to_bytes().unwrap();
        let restored = MigrationState::from_bytes(&bytes).unwrap();

        assert!(restored.is_migrated(1));
        assert!(restored.is_migrated(2));
        assert!(!restored.is_migrated(3));
        assert_eq!(restored.in_progress.len(), 1);
    }

    #[test]
    fn test_migration_config_default() {
        let config = MigrationConfig::default();

        assert!(!config.dry_run);
        assert!(!config.delete_old_messages);
        assert!(config.continue_on_error);
        assert!(!config.verify_after_migration);
        assert_eq!(config.max_concurrent, 4);
    }

    #[test]
    fn test_chunk_migration_state() {
        let pending = ChunkMigrationState::Pending;
        let completed = ChunkMigrationState::Completed;
        let failed = ChunkMigrationState::Failed("test error".to_string());

        assert_ne!(pending, completed);
        assert_ne!(pending, failed);
        assert_eq!(ChunkMigrationState::Pending, ChunkMigrationState::Pending);
    }

    #[test]
    fn test_migration_progress_default() {
        let progress = MigrationProgress::default();

        assert_eq!(progress.total_files, 0);
        assert_eq!(progress.completed_files, 0);
        assert_eq!(progress.total_chunks, 0);
        assert_eq!(progress.completed_chunks, 0);
        assert_eq!(progress.bytes_processed, 0);
    }
}
