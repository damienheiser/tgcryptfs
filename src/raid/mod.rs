//! RAID-style erasure coding across multiple Telegram accounts
//!
//! Provides Reed-Solomon erasure coding with configurable K-of-N recovery.
//! Presets: RAID5 (N-1 of N), RAID6 (N-2 of N), or custom K/N.

pub mod config;
pub mod erasure;
pub mod health;
pub mod migrate;
pub mod pool;
pub mod rebuild;
pub mod stripe;

pub use config::{AccountConfig, ErasureConfig, ErasurePreset, PoolConfig};
pub use erasure::Encoder;
pub use health::{AccountHealth, AccountStatus, ArrayHealth, ArrayStatus, HealthTracker};
pub use migrate::{
    ChunkMigrationState, FileMigrationProgress, LoggingCallback, MigrationCallback,
    MigrationConfig, MigrationManager, MigrationProgress, MigrationState, NoOpCallback,
};
pub use pool::AccountPool;
pub use rebuild::{ProgressCallback, RebuildManager, RebuildPhase, RebuildProgress, ScrubResult};
pub use stripe::{Stripe, StripeManager};
