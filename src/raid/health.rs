//! Account health monitoring for erasure coding pool
//!
//! Tracks the health status of each Telegram account and the overall array.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default number of consecutive failures before marking account unavailable
const DEFAULT_MAX_FAILURES: u32 = 3;

/// Error rate threshold for degraded status (10%)
const DEGRADED_ERROR_RATE_THRESHOLD: f64 = 0.10;

/// Status of a single account
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AccountStatus {
    /// Account is healthy and available
    Healthy,
    /// Account is degraded (high error rate but functional)
    Degraded,
    /// Account is unavailable
    Unavailable,
    /// Account is being rebuilt
    Rebuilding,
}

/// Health information for a single account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountHealth {
    /// Account ID
    pub account_id: u8,
    /// Current status
    pub status: AccountStatus,
    /// Last successful operation timestamp (Unix seconds)
    pub last_success: Option<i64>,
    /// Last error message if any
    pub last_error: Option<String>,
    /// Consecutive failure count
    pub failure_count: u32,
    /// Total operations attempted
    pub total_operations: u64,
    /// Total failed operations
    pub failed_operations: u64,
}

impl AccountHealth {
    /// Create new healthy account
    pub fn new(account_id: u8) -> Self {
        Self {
            account_id,
            status: AccountStatus::Healthy,
            last_success: None,
            last_error: None,
            failure_count: 0,
            total_operations: 0,
            failed_operations: 0,
        }
    }

    /// Calculate error rate
    pub fn error_rate(&self) -> f64 {
        if self.total_operations == 0 {
            return 0.0;
        }
        self.failed_operations as f64 / self.total_operations as f64
    }
}

impl Default for AccountHealth {
    fn default() -> Self {
        Self {
            account_id: 0,
            status: AccountStatus::Healthy,
            last_success: None,
            last_error: None,
            failure_count: 0,
            total_operations: 0,
            failed_operations: 0,
        }
    }
}

/// Overall array status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArrayStatus {
    /// All accounts healthy, full redundancy
    Healthy,
    /// Operating with reduced redundancy (some accounts down but >= K available)
    Degraded,
    /// Cannot operate (fewer than K accounts available)
    Failed,
    /// Rebuild in progress
    Rebuilding,
}

/// Overall RAID array status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayHealth {
    /// Overall status
    pub status: ArrayStatus,
    /// Individual account health
    pub accounts: Vec<AccountHealth>,
    /// Required accounts for operation (K)
    pub required_accounts: usize,
    /// Total accounts configured (N)
    pub total_accounts: usize,
    /// Rebuild progress (0.0 - 1.0) if rebuilding
    pub rebuild_progress: Option<f32>,
}

/// Health tracker for the account pool
pub struct HealthTracker {
    accounts: RwLock<Vec<AccountHealth>>,
    required_accounts: usize,
    max_failures_before_unavailable: u32,
}

impl HealthTracker {
    /// Create a new health tracker
    pub fn new(num_accounts: usize, required_accounts: usize) -> Self {
        let accounts: Vec<AccountHealth> = (0..num_accounts)
            .map(|i| AccountHealth::new(i as u8))
            .collect();

        Self {
            accounts: RwLock::new(accounts),
            required_accounts,
            max_failures_before_unavailable: DEFAULT_MAX_FAILURES,
        }
    }

    /// Create a new health tracker with custom max failures threshold
    pub fn with_max_failures(
        num_accounts: usize,
        required_accounts: usize,
        max_failures: u32,
    ) -> Self {
        let accounts: Vec<AccountHealth> = (0..num_accounts)
            .map(|i| AccountHealth::new(i as u8))
            .collect();

        Self {
            accounts: RwLock::new(accounts),
            required_accounts,
            max_failures_before_unavailable: max_failures,
        }
    }

    /// Get current Unix timestamp in seconds
    fn now_unix_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Record a successful operation for an account
    pub fn record_success(&self, account_id: u8) {
        let mut accounts = self.accounts.write();
        if let Some(account) = accounts.get_mut(account_id as usize) {
            account.total_operations += 1;
            account.last_success = Some(Self::now_unix_secs());
            account.failure_count = 0;

            // Update status based on error rate
            if account.status == AccountStatus::Unavailable {
                // Recovery from unavailable - need explicit reset or rebuild
            } else if account.status != AccountStatus::Rebuilding {
                if account.error_rate() < DEGRADED_ERROR_RATE_THRESHOLD {
                    account.status = AccountStatus::Healthy;
                } else {
                    account.status = AccountStatus::Degraded;
                }
            }
        }
    }

    /// Record a failed operation for an account
    pub fn record_failure(&self, account_id: u8, error: &str) {
        let mut accounts = self.accounts.write();
        if let Some(account) = accounts.get_mut(account_id as usize) {
            account.total_operations += 1;
            account.failed_operations += 1;
            account.failure_count += 1;
            account.last_error = Some(error.to_string());

            // Update status based on failure count
            if account.status != AccountStatus::Rebuilding {
                if account.failure_count >= self.max_failures_before_unavailable {
                    account.status = AccountStatus::Unavailable;
                } else if account.error_rate() >= DEGRADED_ERROR_RATE_THRESHOLD {
                    account.status = AccountStatus::Degraded;
                }
            }
        }
    }

    /// Get health status of a specific account
    pub fn account_health(&self, account_id: u8) -> AccountHealth {
        let accounts = self.accounts.read();
        accounts
            .get(account_id as usize)
            .cloned()
            .unwrap_or_else(|| AccountHealth::new(account_id))
    }

    /// Get overall array health
    pub fn array_health(&self) -> ArrayHealth {
        let accounts = self.accounts.read();
        let healthy_count = accounts
            .iter()
            .filter(|a| a.status == AccountStatus::Healthy || a.status == AccountStatus::Degraded)
            .count();

        let rebuilding = accounts
            .iter()
            .any(|a| a.status == AccountStatus::Rebuilding);

        let rebuild_progress = if rebuilding {
            // Calculate average rebuild progress (placeholder - actual implementation
            // would track real progress)
            Some(0.0)
        } else {
            None
        };

        let status = if rebuilding {
            ArrayStatus::Rebuilding
        } else if healthy_count >= self.required_accounts {
            if healthy_count == accounts.len()
                && accounts.iter().all(|a| a.status == AccountStatus::Healthy)
            {
                ArrayStatus::Healthy
            } else {
                ArrayStatus::Degraded
            }
        } else {
            ArrayStatus::Failed
        };

        ArrayHealth {
            status,
            accounts: accounts.clone(),
            required_accounts: self.required_accounts,
            total_accounts: accounts.len(),
            rebuild_progress,
        }
    }

    /// Check if array is degraded
    pub fn is_degraded(&self) -> bool {
        let health = self.array_health();
        matches!(
            health.status,
            ArrayStatus::Degraded | ArrayStatus::Rebuilding
        )
    }

    /// Check if array can operate (>= K accounts healthy)
    pub fn can_operate(&self) -> bool {
        self.healthy_count() >= self.required_accounts
    }

    /// Get list of healthy account IDs
    pub fn healthy_accounts(&self) -> Vec<u8> {
        let accounts = self.accounts.read();
        accounts
            .iter()
            .filter(|a| a.status == AccountStatus::Healthy || a.status == AccountStatus::Degraded)
            .map(|a| a.account_id)
            .collect()
    }

    /// Get count of healthy accounts
    pub fn healthy_count(&self) -> usize {
        let accounts = self.accounts.read();
        accounts
            .iter()
            .filter(|a| a.status == AccountStatus::Healthy || a.status == AccountStatus::Degraded)
            .count()
    }

    /// Get status of a specific account (convenience method)
    pub fn account_status(&self, account_id: u8) -> AccountStatus {
        self.account_health(account_id).status
    }

    /// Mark account as rebuilding
    pub fn set_rebuilding(&self, account_id: u8) {
        let mut accounts = self.accounts.write();
        if let Some(account) = accounts.get_mut(account_id as usize) {
            account.status = AccountStatus::Rebuilding;
        }
    }

    /// Mark account as healthy after rebuild
    pub fn set_healthy(&self, account_id: u8) {
        let mut accounts = self.accounts.write();
        if let Some(account) = accounts.get_mut(account_id as usize) {
            account.status = AccountStatus::Healthy;
            account.failure_count = 0;
            account.last_error = None;
        }
    }

    /// Reset failure count (e.g., after manual intervention)
    pub fn reset_failures(&self, account_id: u8) {
        let mut accounts = self.accounts.write();
        if let Some(account) = accounts.get_mut(account_id as usize) {
            account.failure_count = 0;
            account.failed_operations = 0;
            account.last_error = None;

            // If was unavailable, move back to healthy
            if account.status == AccountStatus::Unavailable {
                account.status = AccountStatus::Healthy;
            }
        }
    }

    /// Update rebuild progress for an account
    pub fn update_rebuild_progress(&self, _account_id: u8, _progress: f32) {
        // This would update internal tracking of rebuild progress
        // For now, the ArrayHealth calculates it on demand
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_health_new() {
        let health = AccountHealth::new(5);
        assert_eq!(health.account_id, 5);
        assert_eq!(health.status, AccountStatus::Healthy);
        assert_eq!(health.failure_count, 0);
        assert_eq!(health.total_operations, 0);
        assert!(health.last_success.is_none());
        assert!(health.last_error.is_none());
    }

    #[test]
    fn test_account_health_error_rate() {
        let mut health = AccountHealth::new(0);

        // No operations = 0% error rate
        assert_eq!(health.error_rate(), 0.0);

        // 1 failure out of 10 = 10%
        health.total_operations = 10;
        health.failed_operations = 1;
        assert!((health.error_rate() - 0.1).abs() < f64::EPSILON);

        // 5 failures out of 10 = 50%
        health.failed_operations = 5;
        assert!((health.error_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_health_tracker_new() {
        let tracker = HealthTracker::new(5, 3);
        assert_eq!(tracker.healthy_count(), 5);
        assert_eq!(tracker.required_accounts, 3);
        assert!(tracker.can_operate());
    }

    #[test]
    fn test_record_success() {
        let tracker = HealthTracker::new(3, 2);

        tracker.record_success(0);
        let health = tracker.account_health(0);

        assert_eq!(health.status, AccountStatus::Healthy);
        assert_eq!(health.total_operations, 1);
        assert_eq!(health.failure_count, 0);
        assert!(health.last_success.is_some());
    }

    #[test]
    fn test_record_failure() {
        let tracker = HealthTracker::new(3, 2);

        tracker.record_failure(0, "Connection timeout");
        let health = tracker.account_health(0);

        assert_eq!(health.total_operations, 1);
        assert_eq!(health.failed_operations, 1);
        assert_eq!(health.failure_count, 1);
        assert_eq!(health.last_error, Some("Connection timeout".to_string()));
    }

    #[test]
    fn test_status_transitions_to_unavailable() {
        let tracker = HealthTracker::new(3, 2);

        // Default is 3 failures before unavailable
        // First failure: 100% error rate -> Degraded (above 10% threshold)
        tracker.record_failure(0, "Error 1");
        let status = tracker.account_health(0).status;
        assert!(
            status == AccountStatus::Healthy || status == AccountStatus::Degraded,
            "After 1 failure: expected Healthy or Degraded, got {:?}",
            status
        );

        tracker.record_failure(0, "Error 2");
        // May be degraded due to error rate
        let status = tracker.account_health(0).status;
        assert!(
            status == AccountStatus::Healthy || status == AccountStatus::Degraded,
            "After 2 failures: expected Healthy or Degraded, got {:?}",
            status
        );

        tracker.record_failure(0, "Error 3");
        assert_eq!(
            tracker.account_health(0).status,
            AccountStatus::Unavailable
        );
    }

    #[test]
    fn test_failure_count_resets_on_success() {
        let tracker = HealthTracker::new(3, 2);

        tracker.record_failure(0, "Error 1");
        tracker.record_failure(0, "Error 2");
        assert_eq!(tracker.account_health(0).failure_count, 2);

        tracker.record_success(0);
        assert_eq!(tracker.account_health(0).failure_count, 0);
    }

    #[test]
    fn test_degraded_status_on_high_error_rate() {
        let tracker = HealthTracker::new(3, 2);

        // Record many successes first
        for _ in 0..10 {
            tracker.record_success(0);
        }

        // Then record failures to get >10% error rate
        // After 10 successes, need >1.1 failures for >10% rate
        // 2 failures out of 12 total = 16.7%
        tracker.record_failure(0, "Error 1");
        tracker.record_failure(0, "Error 2");

        let health = tracker.account_health(0);
        assert!(health.error_rate() > DEGRADED_ERROR_RATE_THRESHOLD);
        assert_eq!(health.status, AccountStatus::Degraded);
    }

    #[test]
    fn test_array_health_all_healthy() {
        let tracker = HealthTracker::new(5, 3);

        let health = tracker.array_health();
        assert_eq!(health.status, ArrayStatus::Healthy);
        assert_eq!(health.total_accounts, 5);
        assert_eq!(health.required_accounts, 3);
        assert!(health.rebuild_progress.is_none());
    }

    #[test]
    fn test_array_health_degraded() {
        let tracker = HealthTracker::new(5, 3);

        // Make one account unavailable
        for _ in 0..3 {
            tracker.record_failure(0, "Error");
        }

        let health = tracker.array_health();
        assert_eq!(health.status, ArrayStatus::Degraded);
        assert_eq!(tracker.healthy_count(), 4);
        assert!(tracker.can_operate());
    }

    #[test]
    fn test_array_health_failed() {
        let tracker = HealthTracker::new(5, 3);

        // Make 3 accounts unavailable (only 2 left, need 3)
        for account_id in 0..3 {
            for _ in 0..3 {
                tracker.record_failure(account_id, "Error");
            }
        }

        let health = tracker.array_health();
        assert_eq!(health.status, ArrayStatus::Failed);
        assert_eq!(tracker.healthy_count(), 2);
        assert!(!tracker.can_operate());
    }

    #[test]
    fn test_rebuilding_status() {
        let tracker = HealthTracker::new(5, 3);

        // Make one account unavailable then start rebuild
        for _ in 0..3 {
            tracker.record_failure(0, "Error");
        }
        tracker.set_rebuilding(0);

        let health = tracker.array_health();
        assert_eq!(health.status, ArrayStatus::Rebuilding);
        assert!(health.rebuild_progress.is_some());

        // Account should be in rebuilding state
        assert_eq!(
            tracker.account_health(0).status,
            AccountStatus::Rebuilding
        );
    }

    #[test]
    fn test_set_healthy_after_rebuild() {
        let tracker = HealthTracker::new(5, 3);

        // Make unavailable, then rebuild, then healthy
        for _ in 0..3 {
            tracker.record_failure(0, "Error");
        }
        tracker.set_rebuilding(0);
        tracker.set_healthy(0);

        let health = tracker.account_health(0);
        assert_eq!(health.status, AccountStatus::Healthy);
        assert_eq!(health.failure_count, 0);
        assert!(health.last_error.is_none());
    }

    #[test]
    fn test_reset_failures() {
        let tracker = HealthTracker::new(3, 2);

        // Make unavailable
        for _ in 0..3 {
            tracker.record_failure(0, "Error");
        }
        assert_eq!(
            tracker.account_health(0).status,
            AccountStatus::Unavailable
        );

        // Reset failures
        tracker.reset_failures(0);

        let health = tracker.account_health(0);
        assert_eq!(health.status, AccountStatus::Healthy);
        assert_eq!(health.failure_count, 0);
        assert_eq!(health.failed_operations, 0);
        assert!(health.last_error.is_none());
    }

    #[test]
    fn test_healthy_accounts_list() {
        let tracker = HealthTracker::new(5, 3);

        // Make accounts 1 and 3 unavailable
        for _ in 0..3 {
            tracker.record_failure(1, "Error");
            tracker.record_failure(3, "Error");
        }

        let healthy = tracker.healthy_accounts();
        assert_eq!(healthy.len(), 3);
        assert!(healthy.contains(&0));
        assert!(!healthy.contains(&1));
        assert!(healthy.contains(&2));
        assert!(!healthy.contains(&3));
        assert!(healthy.contains(&4));
    }

    #[test]
    fn test_is_degraded() {
        let tracker = HealthTracker::new(5, 3);

        // Initially not degraded
        assert!(!tracker.is_degraded());

        // Make one account unavailable
        for _ in 0..3 {
            tracker.record_failure(0, "Error");
        }

        // Now degraded
        assert!(tracker.is_degraded());
    }

    #[test]
    fn test_custom_max_failures() {
        let tracker = HealthTracker::with_max_failures(3, 2, 5);

        // Should take 5 failures to become unavailable
        for i in 0..4 {
            tracker.record_failure(0, &format!("Error {}", i));
            assert_ne!(
                tracker.account_health(0).status,
                AccountStatus::Unavailable
            );
        }

        tracker.record_failure(0, "Error 5");
        assert_eq!(
            tracker.account_health(0).status,
            AccountStatus::Unavailable
        );
    }

    #[test]
    fn test_account_health_default() {
        let health = AccountHealth::default();
        assert_eq!(health.account_id, 0);
        assert_eq!(health.status, AccountStatus::Healthy);
        assert!(health.last_success.is_none());
        assert!(health.last_error.is_none());
        assert_eq!(health.failure_count, 0);
        assert_eq!(health.total_operations, 0);
        assert_eq!(health.failed_operations, 0);
    }

    #[test]
    fn test_invalid_account_id() {
        let tracker = HealthTracker::new(3, 2);

        // Should not panic, just return default
        let health = tracker.account_health(255);
        assert_eq!(health.account_id, 255);
        assert_eq!(health.status, AccountStatus::Healthy);

        // Recording to invalid account should be safe (no-op)
        tracker.record_success(255);
        tracker.record_failure(255, "Error");
    }
}
