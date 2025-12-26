//! Synchronization daemon for master-replica replication
//!
//! This module implements the background sync loop that:
//! - On master: periodically creates and uploads snapshots
//! - On replica: periodically downloads and applies the latest snapshot

use crate::distributed::replication::{ReplicationRole, SnapshotManager};
use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info};

/// Current synchronization status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    /// Current replication role
    pub role: ReplicationRole,

    /// Last successful sync timestamp
    pub last_sync: Option<DateTime<Utc>>,

    /// Last sync error (if any)
    pub last_error: Option<String>,

    /// Number of successful syncs
    pub sync_count: u64,

    /// Number of failed syncs
    pub error_count: u64,

    /// Current snapshot version
    pub current_version: u64,

    /// Whether sync daemon is running
    pub is_running: bool,

    /// Number of inodes in current state
    pub inode_count: usize,

    /// Last sync duration in milliseconds
    pub last_sync_duration_ms: Option<u64>,
}

impl SyncStatus {
    /// Create a new sync status
    pub fn new(role: ReplicationRole) -> Self {
        Self {
            role,
            last_sync: None,
            last_error: None,
            sync_count: 0,
            error_count: 0,
            current_version: 0,
            is_running: false,
            inode_count: 0,
            last_sync_duration_ms: None,
        }
    }

    /// Mark a successful sync
    pub fn mark_success(&mut self, version: u64, inode_count: usize, duration_ms: u64) {
        self.last_sync = Some(Utc::now());
        self.last_error = None;
        self.sync_count += 1;
        self.current_version = version;
        self.inode_count = inode_count;
        self.last_sync_duration_ms = Some(duration_ms);
    }

    /// Mark a failed sync
    pub fn mark_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.error_count += 1;
    }

    /// Get success rate as a percentage
    pub fn success_rate(&self) -> f64 {
        let total = self.sync_count + self.error_count;
        if total == 0 {
            0.0
        } else {
            (self.sync_count as f64 / total as f64) * 100.0
        }
    }

    /// Check if sync is healthy (has synced recently without errors)
    pub fn is_healthy(&self, max_age_seconds: u64) -> bool {
        if let Some(last_sync) = self.last_sync {
            let age = Utc::now().signed_duration_since(last_sync);
            age.num_seconds() < max_age_seconds as i64 && self.last_error.is_none()
        } else {
            false
        }
    }
}

/// Configuration for sync daemon
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Role of this node
    pub role: ReplicationRole,

    /// Sync interval in seconds
    pub sync_interval_secs: u64,

    /// Number of snapshots to retain
    pub snapshot_retention: usize,

    /// Whether to enable automatic sync
    pub auto_sync_enabled: bool,
}

impl SyncConfig {
    /// Create a master config
    pub fn master(sync_interval_secs: u64, snapshot_retention: usize) -> Self {
        Self {
            role: ReplicationRole::Master,
            sync_interval_secs,
            snapshot_retention,
            auto_sync_enabled: true,
        }
    }

    /// Create a replica config
    pub fn replica(sync_interval_secs: u64) -> Self {
        Self {
            role: ReplicationRole::Replica,
            sync_interval_secs,
            snapshot_retention: 0, // Replicas don't manage snapshots
            auto_sync_enabled: true,
        }
    }
}

/// Background synchronization daemon
///
/// This daemon runs a loop that:
/// - Master: Creates and uploads snapshots at regular intervals
/// - Replica: Downloads and applies latest snapshot at regular intervals
pub struct SyncDaemon {
    /// Snapshot manager
    snapshot_manager: Arc<SnapshotManager>,

    /// Configuration
    config: SyncConfig,

    /// Current status
    status: Arc<RwLock<SyncStatus>>,

    /// Shutdown signal
    shutdown: Arc<RwLock<bool>>,
}

impl SyncDaemon {
    /// Create a new sync daemon
    pub fn new(snapshot_manager: Arc<SnapshotManager>, config: SyncConfig) -> Self {
        let status = SyncStatus::new(config.role);

        Self {
            snapshot_manager,
            config,
            status: Arc::new(RwLock::new(status)),
            shutdown: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the sync daemon in a blocking manner
    ///
    /// This runs the sync loop in the current async context.
    /// It should be spawned as a separate task by the caller if background execution is needed.
    ///
    /// # Example
    /// ```ignore
    /// let daemon = SyncDaemon::new(snapshot_manager, config);
    /// tokio::task::spawn_local(daemon.start());
    /// ```
    pub async fn start(self) {
        let snapshot_manager = self.snapshot_manager.clone();
        let config = self.config.clone();
        let status = self.status.clone();
        let shutdown = self.shutdown.clone();

        info!(
            "Starting sync daemon in {:?} mode (interval: {}s)",
            config.role, config.sync_interval_secs
        );

        // Mark as running
        {
            let mut s = status.write().await;
            s.is_running = true;
        }

        // Create an interval timer
        let mut sync_interval = interval(Duration::from_secs(config.sync_interval_secs));

        loop {
            // Check for shutdown
            if *shutdown.read().await {
                info!("Sync daemon shutting down");
                break;
            }

            // Wait for next tick
            sync_interval.tick().await;

            // Perform sync based on role
            let result = match config.role {
                ReplicationRole::Master => Self::master_sync(&snapshot_manager).await,
                ReplicationRole::Replica => Self::replica_sync(&snapshot_manager).await,
            };

            // Update status
            let mut s = status.write().await;
            match result {
                Ok((version, inode_count, duration_ms)) => {
                    s.mark_success(version, inode_count, duration_ms);
                    info!(
                        "Sync successful: version {}, {} inodes, {}ms",
                        version, inode_count, duration_ms
                    );
                }
                Err(e) => {
                    s.mark_error(e.to_string());
                    error!("Sync failed: {}", e);
                }
            }
        }

        // Mark as not running
        {
            let mut s = status.write().await;
            s.is_running = false;
        }

        info!("Sync daemon stopped");
    }

    /// Perform a master sync (create and upload snapshot)
    async fn master_sync(
        snapshot_manager: &Arc<SnapshotManager>,
    ) -> Result<(u64, usize, u64)> {
        let start = std::time::Instant::now();

        debug!("Master: Creating snapshot");
        let snapshot = snapshot_manager.create_snapshot().await?;

        debug!("Master: Uploading snapshot");
        snapshot_manager.upload_snapshot(&snapshot).await?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok((snapshot.version, snapshot.inode_count(), duration_ms))
    }

    /// Perform a replica sync (download and apply latest snapshot)
    async fn replica_sync(
        snapshot_manager: &Arc<SnapshotManager>,
    ) -> Result<(u64, usize, u64)> {
        let start = std::time::Instant::now();

        debug!("Replica: Downloading latest snapshot");
        let snapshot = snapshot_manager.download_latest_snapshot().await?;

        debug!("Replica: Applying snapshot");
        snapshot_manager.apply_snapshot(&snapshot).await?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok((snapshot.version, snapshot.inode_count(), duration_ms))
    }

    /// Stop the sync daemon
    pub async fn stop(&self) {
        info!("Requesting sync daemon shutdown");
        let mut shutdown = self.shutdown.write().await;
        *shutdown = true;
    }

    /// Get current sync status
    pub async fn get_status(&self) -> SyncStatus {
        self.status.read().await.clone()
    }

    /// Force an immediate sync (regardless of interval)
    pub async fn sync_now(&self) -> Result<()> {
        info!("Manual sync requested");

        let result = match self.config.role {
            ReplicationRole::Master => Self::master_sync(&self.snapshot_manager).await,
            ReplicationRole::Replica => Self::replica_sync(&self.snapshot_manager).await,
        };

        // Update status
        let mut status = self.status.write().await;
        match result {
            Ok((version, inode_count, duration_ms)) => {
                status.mark_success(version, inode_count, duration_ms);
                info!(
                    "Manual sync successful: version {}, {} inodes, {}ms",
                    version, inode_count, duration_ms
                );
                Ok(())
            }
            Err(e) => {
                status.mark_error(e.to_string());
                error!("Manual sync failed: {}", e);
                Err(e)
            }
        }
    }

    /// Wait for the daemon to be healthy (for testing)
    pub async fn wait_for_healthy(&self, timeout_secs: u64) -> bool {
        let start = std::time::Instant::now();

        while start.elapsed().as_secs() < timeout_secs {
            let status = self.get_status().await;
            if status.is_healthy(self.config.sync_interval_secs * 2) {
                return true;
            }
            sleep(Duration::from_secs(1)).await;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_status_creation() {
        let status = SyncStatus::new(ReplicationRole::Master);
        assert_eq!(status.role, ReplicationRole::Master);
        assert_eq!(status.sync_count, 0);
        assert_eq!(status.error_count, 0);
        assert!(status.last_sync.is_none());
        assert!(!status.is_running);
    }

    #[test]
    fn test_sync_status_mark_success() {
        let mut status = SyncStatus::new(ReplicationRole::Master);
        status.mark_success(1, 100, 500);

        assert_eq!(status.sync_count, 1);
        assert_eq!(status.current_version, 1);
        assert_eq!(status.inode_count, 100);
        assert_eq!(status.last_sync_duration_ms, Some(500));
        assert!(status.last_sync.is_some());
        assert!(status.last_error.is_none());
    }

    #[test]
    fn test_sync_status_mark_error() {
        let mut status = SyncStatus::new(ReplicationRole::Replica);
        status.mark_error("Test error".to_string());

        assert_eq!(status.error_count, 1);
        assert_eq!(status.last_error, Some("Test error".to_string()));
    }

    #[test]
    fn test_sync_status_success_rate() {
        let mut status = SyncStatus::new(ReplicationRole::Master);

        // No syncs yet
        assert_eq!(status.success_rate(), 0.0);

        // 3 successes, 1 failure = 75%
        status.mark_success(1, 10, 100);
        status.mark_success(2, 20, 200);
        status.mark_success(3, 30, 300);
        status.mark_error("Error".to_string());

        assert_eq!(status.success_rate(), 75.0);
    }

    #[test]
    fn test_sync_config_master() {
        let config = SyncConfig::master(60, 10);
        assert_eq!(config.role, ReplicationRole::Master);
        assert_eq!(config.sync_interval_secs, 60);
        assert_eq!(config.snapshot_retention, 10);
        assert!(config.auto_sync_enabled);
    }

    #[test]
    fn test_sync_config_replica() {
        let config = SyncConfig::replica(30);
        assert_eq!(config.role, ReplicationRole::Replica);
        assert_eq!(config.sync_interval_secs, 30);
        assert_eq!(config.snapshot_retention, 0);
        assert!(config.auto_sync_enabled);
    }
}
