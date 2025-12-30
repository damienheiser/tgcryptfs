//! Account pool for managing multiple Telegram backends
//!
//! Provides unified interface for uploading/downloading across multiple accounts.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::join_all;
use tracing::{debug, error, info, warn};

use crate::chunk::StripeInfo;
use crate::config::TelegramConfig;
use crate::error::{Error, Result};
use crate::telegram::TelegramBackend;

use super::config::{AccountConfig, PoolConfig};
use super::health::{AccountStatus, ArrayHealth, ArrayStatus, HealthTracker};
use super::stripe::Stripe;

/// Pool of Telegram account backends
pub struct AccountPool {
    /// Individual backends (one per account)
    backends: Vec<Arc<TelegramBackend>>,
    /// Health tracker
    health: Arc<HealthTracker>,
    /// Configuration
    config: PoolConfig,
}

impl AccountPool {
    /// Create a new account pool (does not connect)
    pub fn new(config: PoolConfig) -> Result<Self> {
        // Validate configuration
        config.validate()?;

        let enabled_accounts = config.enabled_accounts();
        if enabled_accounts.is_empty() {
            return Err(Error::InvalidErasureConfig(
                "At least one enabled account is required".to_string(),
            ));
        }

        if enabled_accounts.len() > 255 {
            return Err(Error::InvalidErasureConfig(
                "Maximum 255 accounts supported".to_string(),
            ));
        }

        // Create backends from enabled account configs
        let mut backends = Vec::with_capacity(enabled_accounts.len());
        for account in &enabled_accounts {
            let telegram_config = Self::account_to_telegram_config(account);
            let backend = TelegramBackend::new(telegram_config);
            backends.push(Arc::new(backend));
        }

        // Create health tracker
        let account_count = backends.len();
        let required_accounts = config.erasure.data_chunks;
        let health = Arc::new(HealthTracker::new(account_count, required_accounts));

        info!(
            "Created account pool with {} accounts (K={}, N={})",
            account_count, config.erasure.data_chunks, config.erasure.total_chunks
        );

        Ok(Self {
            backends,
            health,
            config,
        })
    }

    /// Convert AccountConfig to TelegramConfig
    fn account_to_telegram_config(account: &AccountConfig) -> TelegramConfig {
        TelegramConfig {
            api_id: account.api_id,
            api_hash: account.api_hash.clone(),
            phone: account.phone.clone(),
            session_file: account.session_file.clone(),
            max_concurrent_uploads: 3,
            max_concurrent_downloads: 5,
            retry_attempts: 3,
            retry_base_delay_ms: 1000,
        }
    }

    /// Connect all accounts in the pool
    pub async fn connect_all(&self) -> Result<()> {
        info!("Connecting {} accounts...", self.backends.len());

        let connect_futures: Vec<_> = self
            .backends
            .iter()
            .enumerate()
            .map(|(idx, backend)| {
                let backend = Arc::clone(backend);
                let health = Arc::clone(&self.health);
                async move {
                    match backend.connect().await {
                        Ok(()) => {
                            info!("Account {} connected", idx);
                            health.record_success(idx as u8);
                            Ok(())
                        }
                        Err(e) => {
                            error!("Account {} failed to connect: {}", idx, e);
                            health.record_failure(idx as u8, &e.to_string());
                            Err((idx, e))
                        }
                    }
                }
            })
            .collect();

        let results = join_all(connect_futures).await;

        // Count successes and failures
        let mut success_count = 0;
        let mut failures = Vec::new();

        for result in results {
            match result {
                Ok(()) => success_count += 1,
                Err((idx, e)) => failures.push((idx, e)),
            }
        }

        // Check if we have enough connected accounts
        let required = self.config.erasure.data_chunks;
        if success_count < required {
            return Err(Error::ErasureFailed {
                available: success_count,
                required,
            });
        }

        if !failures.is_empty() {
            warn!(
                "Pool connected in degraded mode: {}/{} accounts available",
                success_count,
                self.backends.len()
            );
            for (idx, e) in &failures {
                warn!("  Account {} unavailable: {}", idx, e);
            }
        } else {
            info!("All {} accounts connected successfully", success_count);
        }

        Ok(())
    }

    /// Disconnect all accounts
    pub async fn disconnect_all(&self) {
        info!("Disconnecting {} accounts...", self.backends.len());

        let disconnect_futures: Vec<_> = self
            .backends
            .iter()
            .map(|backend| {
                let backend = Arc::clone(backend);
                async move {
                    backend.disconnect().await;
                }
            })
            .collect();

        join_all(disconnect_futures).await;
        info!("All accounts disconnected");
    }

    /// Get a specific backend by account ID
    pub fn get_backend(&self, account_id: u8) -> Option<Arc<TelegramBackend>> {
        self.backends.get(account_id as usize).map(Arc::clone)
    }

    /// Upload a stripe to all assigned accounts in parallel
    /// Returns StripeInfo with message IDs on success
    /// In degraded mode, uploads to available accounts and warns
    pub async fn upload_stripe(&self, stripe: &Stripe) -> Result<StripeInfo> {
        let all_blocks = stripe.all_blocks();
        let block_count = all_blocks.len();

        debug!(
            "Uploading stripe with {} blocks to {} accounts",
            block_count,
            self.backends.len()
        );

        // Check if we're in degraded mode before starting
        if self.is_degraded() {
            warn!(
                "DEGRADED MODE: Only {}/{} accounts healthy, uploading stripe anyway",
                self.healthy_count(),
                self.backends.len()
            );
        }

        // Create upload futures for each block
        let chunk_id = stripe.chunk_id.clone();
        let upload_futures: Vec<_> = all_blocks
            .into_iter()
            .map(|(block_idx, account_id, data)| {
                let backend = self.get_backend(account_id);
                let health = Arc::clone(&self.health);
                let block_chunk_id = format!("{}_{}", chunk_id, block_idx);
                let data_owned = data.to_vec();

                async move {
                    // Check if this account is unavailable
                    let account_health = health.account_health(account_id);
                    if account_health.status == AccountStatus::Unavailable {
                        warn!(
                            "Skipping upload to unavailable account {} for block {}",
                            account_id, block_idx
                        );
                        return Err((
                            block_idx,
                            account_id,
                            Error::AccountUnavailable(account_id, "Account marked as unavailable".to_string()),
                        ));
                    }

                    let backend = match backend {
                        Some(b) => b,
                        None => {
                            return Err((
                                block_idx,
                                account_id,
                                Error::AccountUnavailable(
                                    account_id,
                                    "Backend not found".to_string(),
                                ),
                            ));
                        }
                    };

                    match backend.upload_chunk(&block_chunk_id, &data_owned).await {
                        Ok(msg_id) => {
                            health.record_success(account_id);
                            debug!(
                                "Block {} uploaded to account {} as message {}",
                                block_idx, account_id, msg_id
                            );
                            Ok((block_idx, account_id, msg_id))
                        }
                        Err(e) => {
                            health.record_failure(account_id, &e.to_string());
                            error!(
                                "Failed to upload block {} to account {}: {}",
                                block_idx, account_id, e
                            );
                            Err((block_idx, account_id, e))
                        }
                    }
                }
            })
            .collect();

        let results = join_all(upload_futures).await;

        // Process results and build StripeInfo
        let data_count = stripe.data_count as u8;
        let parity_count = stripe.parity_count() as u8;
        let block_size = stripe.block_size() as u64;
        let mut stripe_info = StripeInfo::new(data_count, parity_count, block_size);

        let mut success_count = 0;
        let mut failures = Vec::new();

        for result in results {
            match result {
                Ok((block_idx, account_id, msg_id)) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;

                    stripe_info.blocks.push(crate::chunk::BlockLocation {
                        account_id,
                        message_id: Some(msg_id),
                        block_index: block_idx,
                        uploaded_at: Some(now),
                    });
                    success_count += 1;
                }
                Err((block_idx, account_id, e)) => {
                    // Still record the block location but without message_id
                    stripe_info.blocks.push(crate::chunk::BlockLocation {
                        account_id,
                        message_id: None,
                        block_index: block_idx,
                        uploaded_at: None,
                    });
                    failures.push((block_idx, account_id, e));
                }
            }
        }

        // Sort blocks by index for consistency
        stripe_info
            .blocks
            .sort_by_key(|b| (b.block_index, b.account_id));

        // Check if we have enough successful uploads
        let required = self.config.erasure.data_chunks;
        if success_count < required {
            error!(
                "Stripe upload failed: only {}/{} blocks uploaded, need at least {}",
                success_count, block_count, required
            );
            return Err(Error::ErasureFailed {
                available: success_count,
                required,
            });
        }

        if !failures.is_empty() {
            warn!(
                "Stripe uploaded in degraded state: {}/{} blocks successful",
                success_count, block_count
            );
            for (block_idx, account_id, e) in &failures {
                warn!(
                    "  Block {} to account {} failed: {}",
                    block_idx, account_id, e
                );
            }
        } else {
            debug!("Stripe uploaded successfully: {} blocks", success_count);
        }

        Ok(stripe_info)
    }

    /// Download blocks for a stripe from available accounts
    /// Returns Vec of (block_index, data) for successfully downloaded blocks
    pub async fn download_blocks(&self, stripe_info: &StripeInfo) -> Result<Vec<(u8, Vec<u8>)>> {
        debug!(
            "Downloading {} blocks from stripe",
            stripe_info.blocks.len()
        );

        // Filter to blocks that have message IDs (were successfully uploaded)
        let available_blocks: Vec<_> = stripe_info
            .blocks
            .iter()
            .filter(|b| b.message_id.is_some())
            .collect();

        if available_blocks.is_empty() {
            return Err(Error::StripeUnrecoverable {
                available: 0,
                required: stripe_info.data_count as usize,
            });
        }

        // Create download futures
        let download_futures: Vec<_> = available_blocks
            .into_iter()
            .map(|block| {
                let backend = self.get_backend(block.account_id);
                let health = Arc::clone(&self.health);
                let block_idx = block.block_index;
                let account_id = block.account_id;
                let message_id = block.message_id.unwrap(); // Safe: filtered above

                async move {
                    // Check if this account is healthy
                    let account_health = health.account_health(account_id);
                    if account_health.status == AccountStatus::Unavailable {
                        warn!(
                            "Attempting download from unavailable account {} for block {}",
                            account_id, block_idx
                        );
                    }

                    let backend = match backend {
                        Some(b) => b,
                        None => {
                            return Err((
                                block_idx,
                                Error::AccountUnavailable(
                                    account_id,
                                    "Backend not found".to_string(),
                                ),
                            ));
                        }
                    };

                    match backend.download_chunk(message_id).await {
                        Ok(data) => {
                            health.record_success(account_id);
                            debug!(
                                "Block {} downloaded from account {} ({} bytes)",
                                block_idx,
                                account_id,
                                data.len()
                            );
                            Ok((block_idx, data))
                        }
                        Err(e) => {
                            health.record_failure(account_id, &e.to_string());
                            error!(
                                "Failed to download block {} from account {}: {}",
                                block_idx, account_id, e
                            );
                            Err((block_idx, e))
                        }
                    }
                }
            })
            .collect();

        let results = join_all(download_futures).await;

        // Collect successful downloads
        let mut blocks = Vec::new();
        let mut failures = Vec::new();

        for result in results {
            match result {
                Ok((block_idx, data)) => {
                    blocks.push((block_idx, data));
                }
                Err((block_idx, e)) => {
                    failures.push((block_idx, e));
                }
            }
        }

        // Check if we have enough blocks to reconstruct
        let required = stripe_info.data_count as usize;
        if blocks.len() < required {
            error!(
                "Not enough blocks to reconstruct: {}/{} available, need {}",
                blocks.len(),
                stripe_info.blocks.len(),
                required
            );
            return Err(Error::StripeUnrecoverable {
                available: blocks.len(),
                required,
            });
        }

        if !failures.is_empty() {
            warn!(
                "Downloaded {}/{} blocks (failures: {})",
                blocks.len(),
                stripe_info.blocks.len(),
                failures.len()
            );
            for (block_idx, e) in &failures {
                warn!("  Block {} failed: {}", block_idx, e);
            }
        } else {
            debug!("All {} blocks downloaded successfully", blocks.len());
        }

        // Sort by block index for consistent processing
        blocks.sort_by_key(|(idx, _)| *idx);

        Ok(blocks)
    }

    /// Get current array health
    pub fn health(&self) -> ArrayHealth {
        self.health.array_health()
    }

    /// Get the array status (healthy, degraded, failed, rebuilding)
    pub fn status(&self) -> ArrayStatus {
        self.health.array_health().status
    }

    /// Check if pool is degraded
    pub fn is_degraded(&self) -> bool {
        self.health.is_degraded()
    }

    /// Check if pool can operate
    pub fn can_operate(&self) -> bool {
        self.health.can_operate()
    }

    /// Get number of healthy accounts
    pub fn healthy_count(&self) -> usize {
        self.health.healthy_count()
    }

    /// Get list of healthy account IDs
    pub fn healthy_accounts(&self) -> Vec<u8> {
        self.health.healthy_accounts()
    }

    /// Get total number of accounts in pool
    pub fn account_count(&self) -> usize {
        self.backends.len()
    }

    /// Get the pool configuration
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }

    /// Get data chunk count (K)
    pub fn data_chunks(&self) -> usize {
        self.config.erasure.data_chunks
    }

    /// Get total chunk count (N)
    pub fn total_chunks(&self) -> usize {
        self.config.erasure.total_chunks
    }

    /// Get parity chunk count (N-K)
    pub fn parity_chunks(&self) -> usize {
        self.config.erasure.parity_chunks()
    }

    /// Get the health tracker
    pub fn health_tracker(&self) -> &Arc<HealthTracker> {
        &self.health
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::ErasureConfig;
    use std::path::PathBuf;

    fn make_test_config(count: usize) -> PoolConfig {
        let accounts: Vec<AccountConfig> = (0..count)
            .map(|i| AccountConfig::new(
                i as u8,
                12345,
                "test_hash".to_string(),
                PathBuf::from(format!("/tmp/test_session_{}", i)),
            ).with_phone(format!("+1234567890{}", i)))
            .collect();

        let erasure = ErasureConfig::new(3, 5);

        PoolConfig::new(accounts, erasure)
    }

    #[test]
    fn test_pool_creation() {
        let config = make_test_config(5);
        let pool = AccountPool::new(config).unwrap();

        assert_eq!(pool.account_count(), 5);
        assert_eq!(pool.data_chunks(), 3);
        assert_eq!(pool.total_chunks(), 5);
    }

    #[test]
    fn test_pool_empty_accounts() {
        let erasure = ErasureConfig::new(3, 5);
        let config = PoolConfig::new(vec![], erasure);

        let result = AccountPool::new(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_pool_not_enough_accounts() {
        // Only 3 accounts but need N=5
        let accounts: Vec<AccountConfig> = (0..3)
            .map(|i| AccountConfig::new(
                i as u8,
                12345,
                "test_hash".to_string(),
                PathBuf::from(format!("/tmp/test_session_{}", i)),
            ))
            .collect();
        let erasure = ErasureConfig::new(3, 5);
        let config = PoolConfig::new(accounts, erasure);

        let result = AccountPool::new(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_backend() {
        let config = make_test_config(5);
        let pool = AccountPool::new(config).unwrap();

        assert!(pool.get_backend(0).is_some());
        assert!(pool.get_backend(1).is_some());
        assert!(pool.get_backend(2).is_some());
        assert!(pool.get_backend(3).is_some());
        assert!(pool.get_backend(4).is_some());
        assert!(pool.get_backend(5).is_none());
    }

    #[test]
    fn test_pool_health_methods() {
        let config = make_test_config(5);
        let pool = AccountPool::new(config).unwrap();

        // Initially all accounts should be healthy
        assert!(pool.can_operate());
        assert!(!pool.is_degraded());
        assert_eq!(pool.healthy_count(), 5);
        assert_eq!(pool.healthy_accounts().len(), 5);
    }

    #[test]
    fn test_pool_parity_chunks() {
        let config = make_test_config(5);
        let pool = AccountPool::new(config).unwrap();

        assert_eq!(pool.parity_chunks(), 2); // N=5, K=3, parity=2
    }
}
