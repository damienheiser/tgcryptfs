//! Rebuild and scrub operations for erasure-coded data
//!
//! Provides functionality to:
//! - Rebuild data for a failed account from other accounts
//! - Scrub (verify) all stripes for data integrity

use std::sync::Arc;

use futures::stream::{self, StreamExt};
use tracing::{debug, error, info, warn};

use crate::chunk::{ErasureChunkRef, StripeInfo};
use crate::error::{Error, Result};

use super::health::{AccountStatus, HealthTracker};
use super::pool::AccountPool;
use super::stripe::StripeManager;

/// Default batch size for processing stripes during rebuild/scrub
const DEFAULT_BATCH_SIZE: usize = 100;

/// Progress information for rebuild/scrub operations
#[derive(Debug, Clone)]
pub struct RebuildProgress {
    /// Account being rebuilt (None for scrub operations)
    pub account_id: Option<u8>,
    /// Total number of stripes to process
    pub total_stripes: usize,
    /// Number of stripes processed so far
    pub processed_stripes: usize,
    /// Number of stripes successfully rebuilt/verified
    pub successful_stripes: usize,
    /// Number of stripes that failed
    pub failed_stripes: usize,
    /// Current operation phase
    pub phase: RebuildPhase,
}

impl RebuildProgress {
    /// Create new progress tracker
    pub fn new(account_id: Option<u8>, total_stripes: usize) -> Self {
        Self {
            account_id,
            total_stripes,
            processed_stripes: 0,
            successful_stripes: 0,
            failed_stripes: 0,
            phase: RebuildPhase::Starting,
        }
    }

    /// Get progress as a fraction (0.0 to 1.0)
    pub fn progress_fraction(&self) -> f32 {
        if self.total_stripes == 0 {
            return 1.0;
        }
        self.processed_stripes as f32 / self.total_stripes as f32
    }

    /// Get progress as percentage (0 to 100)
    pub fn progress_percent(&self) -> u8 {
        (self.progress_fraction() * 100.0) as u8
    }
}

/// Phase of the rebuild/scrub operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildPhase {
    /// Operation is starting
    Starting,
    /// Scanning for affected stripes
    Scanning,
    /// Rebuilding/verifying stripes
    Processing,
    /// Uploading rebuilt blocks
    Uploading,
    /// Operation completed
    Completed,
    /// Operation failed
    Failed,
}

/// Result from scrubbing a single stripe
#[derive(Debug)]
pub struct ScrubResult {
    /// Stripe identifier (chunk_id)
    pub stripe_id: String,
    /// Whether the stripe passed verification
    pub valid: bool,
    /// Number of blocks that were verified
    pub verified_blocks: usize,
    /// Number of blocks that were missing/failed
    pub missing_blocks: usize,
    /// Error message if verification failed
    pub error: Option<String>,
}

/// Type alias for progress callback
pub type ProgressCallback = Box<dyn Fn(RebuildProgress) + Send + Sync>;

/// Manages rebuild and scrub operations for the RAID pool
pub struct RebuildManager {
    /// Account pool for data access
    pool: Arc<AccountPool>,
    /// Stripe manager for encoding/decoding
    stripe_manager: StripeManager,
    /// Batch size for processing
    batch_size: usize,
}

impl RebuildManager {
    /// Create a new rebuild manager
    ///
    /// # Arguments
    /// * `pool` - The account pool to operate on
    pub fn new(pool: Arc<AccountPool>) -> Result<Self> {
        let data_shards = pool.data_chunks();
        let total_shards = pool.total_chunks();
        let num_accounts = pool.account_count();

        let stripe_manager = StripeManager::new(data_shards, total_shards, num_accounts)?;

        Ok(Self {
            pool,
            stripe_manager,
            batch_size: DEFAULT_BATCH_SIZE,
        })
    }

    /// Set the batch size for processing stripes
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size.max(1);
        self
    }

    /// Get the health tracker from the pool
    pub fn health_tracker(&self) -> &Arc<HealthTracker> {
        self.pool.health_tracker()
    }

    /// Rebuild all data for a failed account
    ///
    /// This operation:
    /// 1. Marks the account as Rebuilding
    /// 2. Iterates through all stripes that have a block on this account
    /// 3. For each stripe, downloads K blocks from other accounts
    /// 4. Uses erasure coding to reconstruct the missing block
    /// 5. Re-uploads the reconstructed block to the account
    /// 6. Marks the account as Healthy when done
    ///
    /// # Arguments
    /// * `account_id` - The account to rebuild
    /// * `stripes` - All stripes that need to be checked/rebuilt
    /// * `progress_callback` - Optional callback for progress updates
    ///
    /// # Returns
    /// * `Ok(())` on successful rebuild
    /// * `Err` if rebuild fails
    pub async fn rebuild_account(
        &self,
        account_id: u8,
        stripes: &[ErasureChunkRef],
        progress_callback: Option<ProgressCallback>,
    ) -> Result<()> {
        info!(
            "Starting rebuild for account {} with {} total stripes",
            account_id,
            stripes.len()
        );

        // Find stripes that have a block on this account
        let affected_stripes: Vec<_> = stripes
            .iter()
            .filter(|s| {
                s.stripe
                    .blocks
                    .iter()
                    .any(|b| b.account_id == account_id)
            })
            .collect();

        let total_stripes = affected_stripes.len();
        info!(
            "Found {} stripes with blocks on account {}",
            total_stripes, account_id
        );

        if total_stripes == 0 {
            info!("No stripes to rebuild for account {}", account_id);
            self.health_tracker().set_healthy(account_id);
            return Ok(());
        }

        // Mark account as rebuilding
        self.health_tracker().set_rebuilding(account_id);

        let mut progress = RebuildProgress::new(Some(account_id), total_stripes);
        progress.phase = RebuildPhase::Processing;

        if let Some(ref cb) = progress_callback {
            cb(progress.clone());
        }

        // Process stripes in batches
        let mut failed_stripes = Vec::new();

        for batch in affected_stripes.chunks(self.batch_size) {
            debug!(
                "Processing batch of {} stripes for account {} rebuild",
                batch.len(),
                account_id
            );

            for stripe_ref in batch {
                match self
                    .rebuild_stripe_for_account(&stripe_ref.stripe, &stripe_ref.id, account_id)
                    .await
                {
                    Ok(()) => {
                        progress.successful_stripes += 1;
                        debug!("Successfully rebuilt stripe {} for account {}", stripe_ref.id, account_id);
                    }
                    Err(e) => {
                        progress.failed_stripes += 1;
                        error!(
                            "Failed to rebuild stripe {} for account {}: {}",
                            stripe_ref.id, account_id, e
                        );
                        failed_stripes.push((stripe_ref.id.clone(), e.to_string()));
                    }
                }

                progress.processed_stripes += 1;

                // Update health tracker with progress
                self.health_tracker()
                    .update_rebuild_progress(account_id, progress.progress_fraction());

                if let Some(ref cb) = progress_callback {
                    cb(progress.clone());
                }
            }
        }

        // Final status
        if failed_stripes.is_empty() {
            progress.phase = RebuildPhase::Completed;
            self.health_tracker().set_healthy(account_id);
            info!(
                "Rebuild completed successfully for account {}: {}/{} stripes rebuilt",
                account_id, progress.successful_stripes, total_stripes
            );

            if let Some(ref cb) = progress_callback {
                cb(progress);
            }

            Ok(())
        } else {
            progress.phase = RebuildPhase::Failed;
            warn!(
                "Rebuild completed with errors for account {}: {}/{} stripes failed",
                account_id,
                failed_stripes.len(),
                total_stripes
            );

            if let Some(ref cb) = progress_callback {
                cb(progress);
            }

            // Keep account in degraded state - don't mark as healthy
            Err(Error::RebuildFailed {
                account: account_id,
                reason: format!(
                    "{} stripes failed to rebuild",
                    failed_stripes.len()
                ),
            })
        }
    }

    /// Rebuild a single stripe for a specific account
    async fn rebuild_stripe_for_account(
        &self,
        stripe_info: &StripeInfo,
        chunk_id: &str,
        target_account_id: u8,
    ) -> Result<()> {
        // Find the block that belongs to this account
        let target_block = stripe_info
            .blocks
            .iter()
            .find(|b| b.account_id == target_account_id)
            .ok_or_else(|| {
                Error::Internal(format!(
                    "No block found for account {} in stripe {}",
                    target_account_id, chunk_id
                ))
            })?;

        // Skip if the block is already present and uploaded
        if target_block.message_id.is_some() {
            // Block exists, check if it's valid by trying to download
            let backend = self.pool.get_backend(target_account_id);
            if let Some(backend) = backend {
                if backend.download_chunk(target_block.message_id.unwrap()).await.is_ok() {
                    debug!(
                        "Block {} on account {} already valid, skipping rebuild",
                        target_block.block_index, target_account_id
                    );
                    return Ok(());
                }
            }
        }

        // Download K blocks from other accounts (excluding the target)
        let other_blocks: Vec<_> = stripe_info
            .blocks
            .iter()
            .filter(|b| b.account_id != target_account_id && b.message_id.is_some())
            .collect();

        if other_blocks.len() < self.pool.data_chunks() {
            return Err(Error::StripeUnrecoverable {
                available: other_blocks.len(),
                required: self.pool.data_chunks(),
            });
        }

        // Download the blocks we need
        let mut downloaded_blocks = Vec::new();
        for block in other_blocks.iter().take(self.pool.data_chunks()) {
            let backend = self.pool.get_backend(block.account_id).ok_or_else(|| {
                Error::AccountUnavailable(block.account_id, "Backend not found".to_string())
            })?;

            // Check account health
            let status = self.health_tracker().account_status(block.account_id);
            if status == AccountStatus::Unavailable {
                continue; // Try another block
            }

            match backend.download_chunk(block.message_id.unwrap()).await {
                Ok(data) => {
                    downloaded_blocks.push((block.block_index, data));
                    self.health_tracker().record_success(block.account_id);
                }
                Err(e) => {
                    self.health_tracker()
                        .record_failure(block.account_id, &e.to_string());
                    warn!(
                        "Failed to download block {} from account {}: {}",
                        block.block_index, block.account_id, e
                    );
                }
            }

            // Check if we have enough blocks
            if downloaded_blocks.len() >= self.pool.data_chunks() {
                break;
            }
        }

        if downloaded_blocks.len() < self.pool.data_chunks() {
            return Err(Error::StripeUnrecoverable {
                available: downloaded_blocks.len(),
                required: self.pool.data_chunks(),
            });
        }

        // Reconstruct all blocks using the stripe manager
        let total_shards = self.stripe_manager.total_shards();
        let mut shards: Vec<Option<Vec<u8>>> = vec![None; total_shards];

        for (block_idx, data) in &downloaded_blocks {
            shards[*block_idx as usize] = Some(data.clone());
        }

        // Decode and re-encode to get all blocks
        let original_data = self.stripe_manager.reconstruct(&downloaded_blocks)?;

        // Re-create the stripe to get the missing block
        let reconstructed_stripe = self
            .stripe_manager
            .create_stripe(chunk_id.to_string(), &original_data, 0)?;

        // Get the block we need
        let target_block_idx = target_block.block_index as usize;
        if target_block_idx >= reconstructed_stripe.blocks.len() {
            return Err(Error::Internal(format!(
                "Block index {} out of range",
                target_block_idx
            )));
        }

        let rebuilt_block_data = &reconstructed_stripe.blocks[target_block_idx];

        // Upload the rebuilt block to the target account
        let backend = self
            .pool
            .get_backend(target_account_id)
            .ok_or_else(|| {
                Error::AccountUnavailable(target_account_id, "Backend not found".to_string())
            })?;

        let block_chunk_id = format!("{}_{}", chunk_id, target_block.block_index);
        match backend.upload_chunk(&block_chunk_id, rebuilt_block_data).await {
            Ok(msg_id) => {
                self.health_tracker().record_success(target_account_id);
                info!(
                    "Rebuilt block {} for account {} as message {}",
                    target_block.block_index, target_account_id, msg_id
                );
                Ok(())
            }
            Err(e) => {
                self.health_tracker()
                    .record_failure(target_account_id, &e.to_string());
                Err(e)
            }
        }
    }

    /// Scrub (verify) all stripes for data integrity
    ///
    /// This operation:
    /// 1. For each stripe, downloads all available blocks
    /// 2. Verifies they can be decoded correctly
    /// 3. Reports any inconsistencies
    ///
    /// # Arguments
    /// * `stripes` - All stripes to verify
    /// * `progress_callback` - Optional callback for progress updates
    ///
    /// # Returns
    /// Vector of scrub results for each stripe
    pub async fn scrub(
        &self,
        stripes: &[ErasureChunkRef],
        progress_callback: Option<ProgressCallback>,
    ) -> Vec<ScrubResult> {
        let total_stripes = stripes.len();
        info!("Starting scrub of {} stripes", total_stripes);

        let mut progress = RebuildProgress::new(None, total_stripes);
        progress.phase = RebuildPhase::Processing;

        if let Some(ref cb) = progress_callback {
            cb(progress.clone());
        }

        let mut results = Vec::with_capacity(total_stripes);

        // Process stripes in batches
        for batch in stripes.chunks(self.batch_size) {
            // Use parallel processing within each batch
            let batch_results: Vec<ScrubResult> = stream::iter(batch)
                .map(|stripe_ref| async {
                    self.scrub_stripe(&stripe_ref.stripe, &stripe_ref.id).await
                })
                .buffer_unordered(self.batch_size.min(10)) // Limit concurrency
                .collect()
                .await;

            for result in batch_results {
                if result.valid {
                    progress.successful_stripes += 1;
                } else {
                    progress.failed_stripes += 1;
                }
                progress.processed_stripes += 1;

                results.push(result);

                if let Some(ref cb) = progress_callback {
                    cb(progress.clone());
                }
            }
        }

        progress.phase = if progress.failed_stripes == 0 {
            RebuildPhase::Completed
        } else {
            RebuildPhase::Failed
        };

        if let Some(ref cb) = progress_callback {
            cb(progress.clone());
        }

        info!(
            "Scrub completed: {}/{} stripes valid, {} failed",
            progress.successful_stripes, total_stripes, progress.failed_stripes
        );

        results
    }

    /// Scrub a single stripe
    async fn scrub_stripe(&self, stripe_info: &StripeInfo, chunk_id: &str) -> ScrubResult {
        let mut verified_blocks = 0;
        let mut missing_blocks = 0;
        let mut downloaded_blocks = Vec::new();

        // Try to download all blocks
        for block in &stripe_info.blocks {
            if block.message_id.is_none() {
                missing_blocks += 1;
                continue;
            }

            let backend = match self.pool.get_backend(block.account_id) {
                Some(b) => b,
                None => {
                    missing_blocks += 1;
                    continue;
                }
            };

            // Check account health
            let status = self.health_tracker().account_status(block.account_id);
            if status == AccountStatus::Unavailable {
                missing_blocks += 1;
                continue;
            }

            match backend.download_chunk(block.message_id.unwrap()).await {
                Ok(data) => {
                    downloaded_blocks.push((block.block_index, data));
                    verified_blocks += 1;
                    self.health_tracker().record_success(block.account_id);
                }
                Err(e) => {
                    missing_blocks += 1;
                    self.health_tracker()
                        .record_failure(block.account_id, &e.to_string());
                    debug!(
                        "Failed to download block {} from account {} for scrub: {}",
                        block.block_index, block.account_id, e
                    );
                }
            }
        }

        // Check if we have enough blocks to reconstruct
        if downloaded_blocks.len() < self.pool.data_chunks() {
            return ScrubResult {
                stripe_id: chunk_id.to_string(),
                valid: false,
                verified_blocks,
                missing_blocks,
                error: Some(format!(
                    "Not enough blocks to verify: have {}, need {}",
                    downloaded_blocks.len(),
                    self.pool.data_chunks()
                )),
            };
        }

        // Try to reconstruct data
        match self.stripe_manager.reconstruct(&downloaded_blocks) {
            Ok(_) => ScrubResult {
                stripe_id: chunk_id.to_string(),
                valid: true,
                verified_blocks,
                missing_blocks,
                error: None,
            },
            Err(e) => ScrubResult {
                stripe_id: chunk_id.to_string(),
                valid: false,
                verified_blocks,
                missing_blocks,
                error: Some(format!("Reconstruction failed: {}", e)),
            },
        }
    }

    /// Get stripes that need repair for a specific account
    ///
    /// Returns stripes where the block for the given account is missing or invalid.
    pub fn stripes_needing_repair<'a>(
        &self,
        stripes: &'a [ErasureChunkRef],
        account_id: u8,
    ) -> Vec<&'a ErasureChunkRef> {
        stripes
            .iter()
            .filter(|s| {
                s.stripe.blocks.iter().any(|b| {
                    b.account_id == account_id && b.message_id.is_none()
                })
            })
            .collect()
    }

    /// Get overall pool health status
    pub fn pool_status(&self) -> super::ArrayStatus {
        self.pool.status()
    }

    /// Check if the pool can perform rebuild operations
    pub fn can_rebuild(&self) -> bool {
        self.pool.can_operate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::BlockLocation;
    use crate::raid::config::{AccountConfig, ErasureConfig, PoolConfig};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_test_config(count: usize) -> PoolConfig {
        let accounts: Vec<AccountConfig> = (0..count)
            .map(|i| {
                AccountConfig::new(
                    i as u8,
                    12345,
                    "test_hash".to_string(),
                    PathBuf::from(format!("/tmp/test_session_{}", i)),
                )
                .with_phone(format!("+1234567890{}", i))
            })
            .collect();

        let erasure = ErasureConfig::new(3, 5);
        PoolConfig::new(accounts, erasure)
    }

    fn make_test_stripe_info(data_count: u8, parity_count: u8) -> StripeInfo {
        let total = data_count + parity_count;
        let mut info = StripeInfo::new(data_count, parity_count, 1024);

        for i in 0..total {
            info.blocks.push(BlockLocation {
                account_id: i,
                message_id: Some(100 + i as i32),
                block_index: i,
                uploaded_at: Some(1234567890),
            });
        }

        info
    }

    fn make_test_erasure_chunk_ref(id: &str, stripe: StripeInfo) -> ErasureChunkRef {
        ErasureChunkRef {
            id: id.to_string(),
            offset: 0,
            original_size: 1024,
            compressed: false,
            stripe,
            version: 1,
        }
    }

    #[test]
    fn test_rebuild_manager_creation() {
        let config = make_test_config(5);
        let pool = Arc::new(AccountPool::new(config).unwrap());
        let manager = RebuildManager::new(pool);

        assert!(manager.is_ok());
    }

    #[test]
    fn test_rebuild_manager_with_batch_size() {
        let config = make_test_config(5);
        let pool = Arc::new(AccountPool::new(config).unwrap());
        let manager = RebuildManager::new(pool).unwrap().with_batch_size(50);

        assert_eq!(manager.batch_size, 50);
    }

    #[test]
    fn test_rebuild_progress_new() {
        let progress = RebuildProgress::new(Some(2), 100);

        assert_eq!(progress.account_id, Some(2));
        assert_eq!(progress.total_stripes, 100);
        assert_eq!(progress.processed_stripes, 0);
        assert_eq!(progress.successful_stripes, 0);
        assert_eq!(progress.failed_stripes, 0);
        assert_eq!(progress.phase, RebuildPhase::Starting);
    }

    #[test]
    fn test_rebuild_progress_fraction() {
        let mut progress = RebuildProgress::new(None, 100);

        // 0%
        assert_eq!(progress.progress_fraction(), 0.0);
        assert_eq!(progress.progress_percent(), 0);

        // 50%
        progress.processed_stripes = 50;
        assert_eq!(progress.progress_fraction(), 0.5);
        assert_eq!(progress.progress_percent(), 50);

        // 100%
        progress.processed_stripes = 100;
        assert_eq!(progress.progress_fraction(), 1.0);
        assert_eq!(progress.progress_percent(), 100);
    }

    #[test]
    fn test_rebuild_progress_empty_total() {
        let progress = RebuildProgress::new(None, 0);
        assert_eq!(progress.progress_fraction(), 1.0);
    }

    #[test]
    fn test_stripes_needing_repair() {
        let config = make_test_config(5);
        let pool = Arc::new(AccountPool::new(config).unwrap());
        let manager = RebuildManager::new(pool).unwrap();

        // Create test stripes
        let mut stripe1 = make_test_stripe_info(3, 2);
        stripe1.blocks[2].message_id = None; // Missing block on account 2

        let stripe2 = make_test_stripe_info(3, 2); // All blocks present

        let mut stripe3 = make_test_stripe_info(3, 2);
        stripe3.blocks[2].message_id = None; // Also missing on account 2

        let stripes = vec![
            make_test_erasure_chunk_ref("chunk1", stripe1),
            make_test_erasure_chunk_ref("chunk2", stripe2),
            make_test_erasure_chunk_ref("chunk3", stripe3),
        ];

        let needing_repair = manager.stripes_needing_repair(&stripes, 2);
        assert_eq!(needing_repair.len(), 2);
        assert_eq!(needing_repair[0].id, "chunk1");
        assert_eq!(needing_repair[1].id, "chunk3");

        // Account 0 has no missing blocks
        let needing_repair = manager.stripes_needing_repair(&stripes, 0);
        assert_eq!(needing_repair.len(), 0);
    }

    #[test]
    fn test_can_rebuild() {
        let config = make_test_config(5);
        let pool = Arc::new(AccountPool::new(config).unwrap());
        let manager = RebuildManager::new(pool).unwrap();

        // Initially all accounts healthy
        assert!(manager.can_rebuild());
    }

    #[test]
    fn test_pool_status() {
        let config = make_test_config(5);
        let pool = Arc::new(AccountPool::new(config).unwrap());
        let manager = RebuildManager::new(pool).unwrap();

        assert_eq!(manager.pool_status(), super::super::ArrayStatus::Healthy);
    }

    #[test]
    fn test_scrub_result() {
        let result = ScrubResult {
            stripe_id: "test_stripe".to_string(),
            valid: true,
            verified_blocks: 5,
            missing_blocks: 0,
            error: None,
        };

        assert!(result.valid);
        assert_eq!(result.verified_blocks, 5);
        assert_eq!(result.missing_blocks, 0);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_progress_callback_integration() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();

        let callback: ProgressCallback = Box::new(move |progress: RebuildProgress| {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
            assert!(progress.progress_fraction() >= 0.0);
            assert!(progress.progress_fraction() <= 1.0);
        });

        // Simulate callback usage
        let progress = RebuildProgress::new(Some(0), 10);
        callback(progress);

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_rebuild_phase_transitions() {
        let mut progress = RebuildProgress::new(Some(0), 100);

        assert_eq!(progress.phase, RebuildPhase::Starting);

        progress.phase = RebuildPhase::Scanning;
        assert_eq!(progress.phase, RebuildPhase::Scanning);

        progress.phase = RebuildPhase::Processing;
        assert_eq!(progress.phase, RebuildPhase::Processing);

        progress.phase = RebuildPhase::Uploading;
        assert_eq!(progress.phase, RebuildPhase::Uploading);

        progress.phase = RebuildPhase::Completed;
        assert_eq!(progress.phase, RebuildPhase::Completed);
    }

    #[test]
    fn test_rebuild_manager_health_tracker_access() {
        let config = make_test_config(5);
        let pool = Arc::new(AccountPool::new(config).unwrap());
        let manager = RebuildManager::new(pool).unwrap();

        let health = manager.health_tracker();
        assert_eq!(health.healthy_count(), 5);
    }
}
