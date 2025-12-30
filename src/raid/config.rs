//! Configuration types for RAID-style erasure coding
//!
//! Defines erasure coding presets, account configuration, and pool settings
//! for distributing data across multiple Telegram accounts.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Erasure coding preset configurations
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ErasurePreset {
    /// RAID5-style: Can tolerate 1 account failure (N-1 of N)
    Raid5,

    /// RAID6-style: Can tolerate 2 account failures (N-2 of N)
    Raid6,

    /// Custom K-of-N configuration
    Custom,
}

impl Default for ErasurePreset {
    fn default() -> Self {
        ErasurePreset::Raid5
    }
}

/// Erasure coding configuration
///
/// Defines the Reed-Solomon parameters for data distribution:
/// - `data_chunks` (K): Number of data chunks required to reconstruct
/// - `total_chunks` (N): Total number of chunks including parity
///
/// Any K chunks out of N can reconstruct the original data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErasureConfig {
    /// Number of data chunks required to reconstruct (K)
    pub data_chunks: usize,

    /// Total number of chunks including parity (N)
    pub total_chunks: usize,

    /// Preset used for this configuration
    #[serde(default)]
    pub preset: ErasurePreset,

    /// Whether erasure coding is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for ErasureConfig {
    fn default() -> Self {
        ErasureConfig {
            data_chunks: 2,
            total_chunks: 3,
            preset: ErasurePreset::Raid5,
            enabled: true,
        }
    }
}

impl ErasureConfig {
    /// Create a new erasure config with custom K and N values
    pub fn new(data_chunks: usize, total_chunks: usize) -> Self {
        ErasureConfig {
            data_chunks,
            total_chunks,
            preset: ErasurePreset::Custom,
            enabled: true,
        }
    }

    /// Create an erasure config from a preset and number of accounts
    ///
    /// # Arguments
    /// * `preset` - The erasure preset to use
    /// * `num_accounts` - Number of available accounts (determines N)
    ///
    /// # Returns
    /// * `Ok(ErasureConfig)` - Valid configuration
    /// * `Err` - If insufficient accounts for the preset
    pub fn from_preset(preset: ErasurePreset, num_accounts: usize) -> Result<Self> {
        let (data_chunks, total_chunks) = match preset {
            ErasurePreset::Raid5 => {
                // RAID5: Can lose 1 account, so K = N - 1
                if num_accounts < 2 {
                    return Err(Error::InvalidConfig(
                        "RAID5 requires at least 2 accounts".to_string(),
                    ));
                }
                (num_accounts - 1, num_accounts)
            }
            ErasurePreset::Raid6 => {
                // RAID6: Can lose 2 accounts, so K = N - 2
                if num_accounts < 3 {
                    return Err(Error::InvalidConfig(
                        "RAID6 requires at least 3 accounts".to_string(),
                    ));
                }
                (num_accounts - 2, num_accounts)
            }
            ErasurePreset::Custom => {
                return Err(Error::InvalidConfig(
                    "Custom preset requires explicit K and N values".to_string(),
                ));
            }
        };

        Ok(ErasureConfig {
            data_chunks,
            total_chunks,
            preset,
            enabled: true,
        })
    }

    /// Validate the erasure configuration
    ///
    /// # Validation Rules
    /// - K (data_chunks) must be >= 1
    /// - N (total_chunks) must be >= 2
    /// - K must be < N (need at least one parity chunk)
    pub fn validate(&self) -> Result<()> {
        if self.data_chunks < 1 {
            return Err(Error::InvalidConfig(
                "data_chunks (K) must be at least 1".to_string(),
            ));
        }

        if self.total_chunks < 2 {
            return Err(Error::InvalidConfig(
                "total_chunks (N) must be at least 2".to_string(),
            ));
        }

        if self.data_chunks >= self.total_chunks {
            return Err(Error::InvalidConfig(format!(
                "data_chunks (K={}) must be less than total_chunks (N={})",
                self.data_chunks, self.total_chunks
            )));
        }

        Ok(())
    }

    /// Get the number of parity chunks
    pub fn parity_chunks(&self) -> usize {
        self.total_chunks - self.data_chunks
    }

    /// Get the maximum number of failures that can be tolerated
    pub fn fault_tolerance(&self) -> usize {
        self.parity_chunks()
    }
}

/// Configuration for a single Telegram account in the pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Unique identifier for this account within the pool (0-255)
    pub account_id: u8,

    /// Telegram API ID (get from my.telegram.org)
    pub api_id: i32,

    /// Telegram API hash
    pub api_hash: String,

    /// Phone number for authentication (optional, can be provided at runtime)
    pub phone: Option<String>,

    /// Session file path for this account
    pub session_file: PathBuf,

    /// Priority for chunk distribution (higher = preferred, 0-255)
    #[serde(default = "default_priority")]
    pub priority: u8,

    /// Whether this account is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_priority() -> u8 {
    100
}

impl AccountConfig {
    /// Create a new account configuration
    pub fn new(
        account_id: u8,
        api_id: i32,
        api_hash: String,
        session_file: PathBuf,
    ) -> Self {
        AccountConfig {
            account_id,
            api_id,
            api_hash,
            phone: None,
            session_file,
            priority: default_priority(),
            enabled: true,
        }
    }

    /// Set the phone number
    pub fn with_phone(mut self, phone: String) -> Self {
        self.phone = Some(phone);
        self
    }

    /// Set the priority
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Disable this account
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Pool configuration for managing multiple Telegram accounts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    /// List of account configurations
    pub accounts: Vec<AccountConfig>,

    /// Erasure coding configuration
    pub erasure: ErasureConfig,

    /// Maximum concurrent uploads across all accounts
    #[serde(default = "default_max_concurrent_uploads")]
    pub max_concurrent_uploads: usize,

    /// Maximum concurrent downloads across all accounts
    #[serde(default = "default_max_concurrent_downloads")]
    pub max_concurrent_downloads: usize,

    /// Number of retry attempts for failed operations
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: u32,

    /// Health check interval in seconds
    #[serde(default = "default_health_check_interval")]
    pub health_check_interval_secs: u64,
}

fn default_max_concurrent_uploads() -> usize {
    6
}

fn default_max_concurrent_downloads() -> usize {
    10
}

fn default_retry_attempts() -> u32 {
    3
}

fn default_health_check_interval() -> u64 {
    300 // 5 minutes
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            accounts: Vec::new(),
            erasure: ErasureConfig::default(),
            max_concurrent_uploads: default_max_concurrent_uploads(),
            max_concurrent_downloads: default_max_concurrent_downloads(),
            retry_attempts: default_retry_attempts(),
            health_check_interval_secs: default_health_check_interval(),
        }
    }
}

impl PoolConfig {
    /// Create a new pool configuration
    pub fn new(accounts: Vec<AccountConfig>, erasure: ErasureConfig) -> Self {
        PoolConfig {
            accounts,
            erasure,
            ..Default::default()
        }
    }

    /// Validate the pool configuration
    ///
    /// # Validation Rules
    /// - Erasure config must be valid
    /// - Must have enough enabled accounts for total_chunks (N)
    /// - Account IDs must be unique
    /// - All enabled accounts must have valid API credentials
    pub fn validate(&self) -> Result<()> {
        // Validate erasure config first
        self.erasure.validate()?;

        // Count enabled accounts
        let enabled_accounts: Vec<_> = self.accounts.iter().filter(|a| a.enabled).collect();
        let enabled_count = enabled_accounts.len();

        if enabled_count < self.erasure.total_chunks {
            return Err(Error::InvalidConfig(format!(
                "Not enough enabled accounts: have {}, need {} for N={}",
                enabled_count,
                self.erasure.total_chunks,
                self.erasure.total_chunks
            )));
        }

        // Check for unique account IDs
        let mut seen_ids = std::collections::HashSet::new();
        for account in &self.accounts {
            if !seen_ids.insert(account.account_id) {
                return Err(Error::InvalidConfig(format!(
                    "Duplicate account_id: {}",
                    account.account_id
                )));
            }
        }

        // Validate each enabled account
        for account in enabled_accounts {
            if account.api_id == 0 {
                return Err(Error::InvalidConfig(format!(
                    "Account {} has invalid api_id (0)",
                    account.account_id
                )));
            }
            if account.api_hash.is_empty() {
                return Err(Error::InvalidConfig(format!(
                    "Account {} has empty api_hash",
                    account.account_id
                )));
            }
        }

        Ok(())
    }

    /// Get all enabled accounts sorted by priority (highest first)
    pub fn enabled_accounts(&self) -> Vec<&AccountConfig> {
        let mut accounts: Vec<_> = self.accounts.iter().filter(|a| a.enabled).collect();
        accounts.sort_by(|a, b| b.priority.cmp(&a.priority));
        accounts
    }

    /// Get an account by ID
    pub fn get_account(&self, account_id: u8) -> Option<&AccountConfig> {
        self.accounts.iter().find(|a| a.account_id == account_id)
    }

    /// Get a mutable reference to an account by ID
    pub fn get_account_mut(&mut self, account_id: u8) -> Option<&mut AccountConfig> {
        self.accounts.iter_mut().find(|a| a.account_id == account_id)
    }

    /// Add a new account to the pool
    pub fn add_account(&mut self, account: AccountConfig) -> Result<()> {
        if self.accounts.iter().any(|a| a.account_id == account.account_id) {
            return Err(Error::InvalidConfig(format!(
                "Account with id {} already exists",
                account.account_id
            )));
        }
        self.accounts.push(account);
        Ok(())
    }

    /// Remove an account from the pool by ID
    pub fn remove_account(&mut self, account_id: u8) -> Option<AccountConfig> {
        if let Some(pos) = self.accounts.iter().position(|a| a.account_id == account_id) {
            Some(self.accounts.remove(pos))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_erasure_config_validation() {
        // Valid config
        let config = ErasureConfig::new(2, 3);
        assert!(config.validate().is_ok());

        // K must be >= 1
        let config = ErasureConfig::new(0, 3);
        assert!(config.validate().is_err());

        // N must be >= 2
        let config = ErasureConfig::new(1, 1);
        assert!(config.validate().is_err());

        // K must be < N
        let config = ErasureConfig::new(3, 3);
        assert!(config.validate().is_err());

        let config = ErasureConfig::new(4, 3);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_erasure_from_preset_raid5() {
        // RAID5 with 3 accounts: K=2, N=3
        let config = ErasureConfig::from_preset(ErasurePreset::Raid5, 3).unwrap();
        assert_eq!(config.data_chunks, 2);
        assert_eq!(config.total_chunks, 3);
        assert_eq!(config.fault_tolerance(), 1);

        // RAID5 with 5 accounts: K=4, N=5
        let config = ErasureConfig::from_preset(ErasurePreset::Raid5, 5).unwrap();
        assert_eq!(config.data_chunks, 4);
        assert_eq!(config.total_chunks, 5);

        // RAID5 needs at least 2 accounts
        assert!(ErasureConfig::from_preset(ErasurePreset::Raid5, 1).is_err());
    }

    #[test]
    fn test_erasure_from_preset_raid6() {
        // RAID6 with 4 accounts: K=2, N=4
        let config = ErasureConfig::from_preset(ErasurePreset::Raid6, 4).unwrap();
        assert_eq!(config.data_chunks, 2);
        assert_eq!(config.total_chunks, 4);
        assert_eq!(config.fault_tolerance(), 2);

        // RAID6 needs at least 3 accounts
        assert!(ErasureConfig::from_preset(ErasurePreset::Raid6, 2).is_err());
    }

    #[test]
    fn test_pool_config_validation() {
        let accounts = vec![
            AccountConfig::new(0, 12345, "hash1".to_string(), PathBuf::from("session0")),
            AccountConfig::new(1, 12346, "hash2".to_string(), PathBuf::from("session1")),
            AccountConfig::new(2, 12347, "hash3".to_string(), PathBuf::from("session2")),
        ];

        let erasure = ErasureConfig::new(2, 3);
        let pool = PoolConfig::new(accounts, erasure);
        assert!(pool.validate().is_ok());
    }

    #[test]
    fn test_pool_config_not_enough_accounts() {
        let accounts = vec![
            AccountConfig::new(0, 12345, "hash1".to_string(), PathBuf::from("session0")),
            AccountConfig::new(1, 12346, "hash2".to_string(), PathBuf::from("session1")),
        ];

        // Need 4 accounts but only have 2
        let erasure = ErasureConfig::new(3, 4);
        let pool = PoolConfig::new(accounts, erasure);
        assert!(pool.validate().is_err());
    }

    #[test]
    fn test_pool_config_duplicate_account_ids() {
        let accounts = vec![
            AccountConfig::new(0, 12345, "hash1".to_string(), PathBuf::from("session0")),
            AccountConfig::new(0, 12346, "hash2".to_string(), PathBuf::from("session1")), // Duplicate ID
            AccountConfig::new(2, 12347, "hash3".to_string(), PathBuf::from("session2")),
        ];

        let erasure = ErasureConfig::new(2, 3);
        let pool = PoolConfig::new(accounts, erasure);
        assert!(pool.validate().is_err());
    }

    #[test]
    fn test_enabled_accounts_sorted_by_priority() {
        let accounts = vec![
            AccountConfig::new(0, 12345, "hash1".to_string(), PathBuf::from("session0"))
                .with_priority(50),
            AccountConfig::new(1, 12346, "hash2".to_string(), PathBuf::from("session1"))
                .with_priority(200),
            AccountConfig::new(2, 12347, "hash3".to_string(), PathBuf::from("session2"))
                .with_priority(100),
        ];

        let erasure = ErasureConfig::new(2, 3);
        let pool = PoolConfig::new(accounts, erasure);

        let enabled = pool.enabled_accounts();
        assert_eq!(enabled[0].account_id, 1); // Priority 200
        assert_eq!(enabled[1].account_id, 2); // Priority 100
        assert_eq!(enabled[2].account_id, 0); // Priority 50
    }

    #[test]
    fn test_disabled_accounts_not_counted() {
        let accounts = vec![
            AccountConfig::new(0, 12345, "hash1".to_string(), PathBuf::from("session0")),
            AccountConfig::new(1, 12346, "hash2".to_string(), PathBuf::from("session1")).disabled(),
            AccountConfig::new(2, 12347, "hash3".to_string(), PathBuf::from("session2")),
        ];

        // Need 3 accounts but only 2 are enabled
        let erasure = ErasureConfig::new(2, 3);
        let pool = PoolConfig::new(accounts, erasure);
        assert!(pool.validate().is_err());
    }
}
