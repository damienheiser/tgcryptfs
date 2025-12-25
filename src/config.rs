//! Configuration management for TelegramFS

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default chunk size: 50MB (safe margin under Telegram's 2GB limit)
pub const DEFAULT_CHUNK_SIZE: usize = 50 * 1024 * 1024;

/// Default cache size: 1GB
pub const DEFAULT_CACHE_SIZE: u64 = 1024 * 1024 * 1024;

/// Default prefetch count
pub const DEFAULT_PREFETCH_COUNT: usize = 3;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Telegram API configuration
    pub telegram: TelegramConfig,

    /// Encryption configuration
    pub encryption: EncryptionConfig,

    /// Cache configuration
    pub cache: CacheConfig,

    /// Chunk configuration
    pub chunk: ChunkConfig,

    /// Mount configuration
    pub mount: MountConfig,

    /// Version control configuration
    pub versioning: VersioningConfig,

    /// Path to the data directory
    pub data_dir: PathBuf,
}

/// Telegram API configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Telegram API ID (get from my.telegram.org)
    pub api_id: i32,

    /// Telegram API hash
    pub api_hash: String,

    /// Phone number for authentication
    pub phone: Option<String>,

    /// Session file path
    pub session_file: PathBuf,

    /// Maximum concurrent uploads
    pub max_concurrent_uploads: usize,

    /// Maximum concurrent downloads
    pub max_concurrent_downloads: usize,

    /// Retry attempts for failed operations
    pub retry_attempts: u32,

    /// Base delay for exponential backoff (ms)
    pub retry_base_delay_ms: u64,
}

/// Encryption configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    /// Argon2 memory cost in KiB
    pub argon2_memory_kib: u32,

    /// Argon2 time cost (iterations)
    pub argon2_iterations: u32,

    /// Argon2 parallelism
    pub argon2_parallelism: u32,

    /// Salt for key derivation (will be generated if not set)
    #[serde(with = "hex_serde")]
    pub salt: Vec<u8>,
}

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Maximum cache size in bytes
    pub max_size: u64,

    /// Cache directory path
    pub cache_dir: PathBuf,

    /// Enable prefetching
    pub prefetch_enabled: bool,

    /// Number of chunks to prefetch
    pub prefetch_count: usize,

    /// Cache eviction policy
    pub eviction_policy: EvictionPolicy,
}

/// Chunk configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkConfig {
    /// Target chunk size in bytes
    pub chunk_size: usize,

    /// Enable compression
    pub compression_enabled: bool,

    /// Minimum size to compress (bytes)
    pub compression_threshold: usize,

    /// Enable content-based deduplication
    pub dedup_enabled: bool,
}

/// Mount configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    /// Mount point path
    pub mount_point: PathBuf,

    /// Allow other users to access the mount
    pub allow_other: bool,

    /// Allow root to access the mount
    pub allow_root: bool,

    /// Default file permissions
    pub default_file_mode: u32,

    /// Default directory permissions
    pub default_dir_mode: u32,

    /// UID for files
    pub uid: u32,

    /// GID for files
    pub gid: u32,
}

/// Versioning configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersioningConfig {
    /// Enable version history
    pub enabled: bool,

    /// Maximum versions to keep per file (0 = unlimited)
    pub max_versions: usize,

    /// Enable automatic snapshots
    pub auto_snapshot: bool,

    /// Snapshot interval in seconds (0 = disabled)
    pub snapshot_interval_secs: u64,

    /// Maximum snapshots to keep (0 = unlimited)
    pub max_snapshots: usize,
}

/// Cache eviction policy
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used
    Lru,
    /// Least Frequently Used
    Lfu,
    /// First In First Out
    Fifo,
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("telegramfs");

        Config {
            telegram: TelegramConfig::default(),
            encryption: EncryptionConfig::default(),
            cache: CacheConfig {
                max_size: DEFAULT_CACHE_SIZE,
                cache_dir: data_dir.join("cache"),
                prefetch_enabled: true,
                prefetch_count: DEFAULT_PREFETCH_COUNT,
                eviction_policy: EvictionPolicy::Lru,
            },
            chunk: ChunkConfig::default(),
            mount: MountConfig::default(),
            versioning: VersioningConfig::default(),
            data_dir,
        }
    }
}

impl Default for TelegramConfig {
    fn default() -> Self {
        TelegramConfig {
            api_id: 0,
            api_hash: String::new(),
            phone: None,
            session_file: PathBuf::from("telegramfs.session"),
            max_concurrent_uploads: 3,
            max_concurrent_downloads: 5,
            retry_attempts: 3,
            retry_base_delay_ms: 1000,
        }
    }
}

impl Default for EncryptionConfig {
    fn default() -> Self {
        EncryptionConfig {
            argon2_memory_kib: 65536,  // 64 MiB
            argon2_iterations: 3,
            argon2_parallelism: 4,
            salt: Vec::new(), // Will be generated on first use
        }
    }
}

impl Default for ChunkConfig {
    fn default() -> Self {
        ChunkConfig {
            chunk_size: DEFAULT_CHUNK_SIZE,
            compression_enabled: true,
            compression_threshold: 1024, // Only compress if > 1KB
            dedup_enabled: true,
        }
    }
}

impl Default for MountConfig {
    fn default() -> Self {
        MountConfig {
            mount_point: PathBuf::from("/mnt/telegramfs"),
            allow_other: false,
            allow_root: false,
            default_file_mode: 0o644,
            default_dir_mode: 0o755,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        }
    }
}

impl Default for VersioningConfig {
    fn default() -> Self {
        VersioningConfig {
            enabled: true,
            max_versions: 10,
            auto_snapshot: false,
            snapshot_interval_secs: 0,
            max_snapshots: 5,
        }
    }
}

impl Config {
    /// Load configuration from a file, with environment variable overrides
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            Error::Config(format!("Failed to read config file: {}", e))
        })?;

        let mut config: Config = serde_json::from_str(&content).map_err(|e| {
            Error::Config(format!("Failed to parse config file: {}", e))
        })?;

        // Override with environment variables if set
        config.apply_env_overrides();

        config.validate()?;
        Ok(config)
    }

    /// Apply environment variable overrides to configuration
    pub fn apply_env_overrides(&mut self) {
        // Telegram credentials from environment
        if let Ok(api_id) = std::env::var("TELEGRAM_APP_ID") {
            if let Ok(id) = api_id.trim().parse::<i32>() {
                self.telegram.api_id = id;
            }
        }

        if let Ok(api_hash) = std::env::var("TELEGRAM_APP_HASH") {
            let hash = api_hash.trim().to_string();
            if !hash.is_empty() {
                self.telegram.api_hash = hash;
            }
        }

        if let Ok(phone) = std::env::var("TELEGRAM_PHONE") {
            let phone = phone.trim().to_string();
            if !phone.is_empty() {
                self.telegram.phone = Some(phone);
            }
        }

        // Cache settings
        if let Ok(cache_size) = std::env::var("TELEGRAMFS_CACHE_SIZE") {
            if let Ok(size) = cache_size.trim().parse::<u64>() {
                self.cache.max_size = size;
            }
        }

        // Chunk settings
        if let Ok(chunk_size) = std::env::var("TELEGRAMFS_CHUNK_SIZE") {
            if let Ok(size) = chunk_size.trim().parse::<usize>() {
                self.chunk.chunk_size = size;
            }
        }
    }

    /// Create a new config from environment variables only (for init without existing config)
    pub fn from_env() -> Result<Self> {
        let mut config = Config::default();
        config.apply_env_overrides();

        // For from_env, we require API credentials
        if config.telegram.api_id == 0 {
            return Err(Error::InvalidConfig(
                "TELEGRAM_APP_ID environment variable is required".to_string(),
            ));
        }
        if config.telegram.api_hash.is_empty() {
            return Err(Error::InvalidConfig(
                "TELEGRAM_APP_HASH environment variable is required".to_string(),
            ));
        }

        Ok(config)
    }

    /// Save configuration to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            Error::Config(format!("Failed to serialize config: {}", e))
        })?;

        std::fs::write(path.as_ref(), content).map_err(|e| {
            Error::Config(format!("Failed to write config file: {}", e))
        })?;

        Ok(())
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.telegram.api_id == 0 {
            return Err(Error::InvalidConfig(
                "Telegram API ID is required".to_string(),
            ));
        }

        if self.telegram.api_hash.is_empty() {
            return Err(Error::InvalidConfig(
                "Telegram API hash is required".to_string(),
            ));
        }

        if self.chunk.chunk_size == 0 {
            return Err(Error::InvalidConfig(
                "Chunk size must be greater than 0".to_string(),
            ));
        }

        if self.chunk.chunk_size > 2 * 1024 * 1024 * 1024 {
            return Err(Error::InvalidConfig(
                "Chunk size exceeds Telegram's 2GB limit".to_string(),
            ));
        }

        Ok(())
    }

    /// Ensure all required directories exist
    pub fn ensure_directories(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache.cache_dir)?;
        Ok(())
    }
}

/// Hex serialization for byte arrays
mod hex_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.is_empty() {
            return Ok(Vec::new());
        }
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}
