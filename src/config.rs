//! Configuration management for tgcryptfs

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Compile-time embedded Telegram API ID.
/// Set via `TGCRYPTFS_DEFAULT_API_ID` environment variable at build time.
/// This allows building releases with embedded credentials for usage tracking.
/// Users can still override at runtime with `TELEGRAM_APP_ID` environment variable.
pub const EMBEDDED_API_ID: Option<&str> = option_env!("TGCRYPTFS_DEFAULT_API_ID");

/// Compile-time embedded Telegram API Hash.
/// Set via `TGCRYPTFS_DEFAULT_API_HASH` environment variable at build time.
/// This allows building releases with embedded credentials for usage tracking.
/// Users can still override at runtime with `TELEGRAM_APP_HASH` environment variable.
pub const EMBEDDED_API_HASH: Option<&str> = option_env!("TGCRYPTFS_DEFAULT_API_HASH");

/// Check if this binary has embedded Telegram API credentials.
///
/// Returns true if both API ID and API Hash were set at compile time.
/// This can be used to inform users whether they need to provide their own credentials.
pub fn has_embedded_credentials() -> bool {
    EMBEDDED_API_ID.is_some() && EMBEDDED_API_HASH.is_some()
}

/// Default chunk size: 50MB (safe margin under cloud storage limit)
pub const DEFAULT_CHUNK_SIZE: usize = 50 * 1024 * 1024;

/// Default cache size: 1GB
pub const DEFAULT_CACHE_SIZE: u64 = 1024 * 1024 * 1024;

/// Default prefetch count
pub const DEFAULT_PREFETCH_COUNT: usize = 3;

/// Default sync interval for master-replica (seconds)
pub const DEFAULT_MASTER_REPLICA_SYNC_INTERVAL: u64 = 60;

/// Default sync interval for distributed mode (milliseconds)
pub const DEFAULT_DISTRIBUTED_SYNC_INTERVAL: u64 = 1000;

/// Machine identity configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineConfig {
    /// Machine ID (UUID or human-readable name)
    pub id: String,

    /// Human-readable machine name
    pub name: String,
}

/// Distribution mode
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DistributionMode {
    /// Single machine, independent filesystem
    Standalone,

    /// One writer, multiple readers with sync
    MasterReplica,

    /// Full read/write from any node with CRDT
    Distributed,
}

/// Distribution configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionConfig {
    /// Distribution mode
    pub mode: DistributionMode,

    /// Cluster ID (for master-replica and distributed modes)
    pub cluster_id: Option<String>,

    /// Master-replica specific configuration
    pub master_replica: Option<MasterReplicaConfig>,

    /// Distributed CRDT configuration
    pub distributed: Option<DistributedConfig>,
}

/// Master-replica configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterReplicaConfig {
    /// Role in the cluster
    pub role: ReplicaRole,

    /// Master machine ID
    pub master_id: String,

    /// Sync interval in seconds
    pub sync_interval_secs: u64,

    /// Number of snapshots to retain
    #[serde(default = "default_snapshot_retention")]
    pub snapshot_retention: usize,
}

/// Replica role
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReplicaRole {
    /// Master (read/write)
    Master,

    /// Replica (read-only)
    Replica,
}

fn default_snapshot_retention() -> usize {
    10
}

/// Distributed CRDT configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedConfig {
    /// Sync interval in milliseconds
    pub sync_interval_ms: u64,

    /// Conflict resolution strategy
    pub conflict_resolution: ConflictResolution,

    /// Operation log retention in hours
    #[serde(default = "default_op_retention")]
    pub operation_log_retention_hours: u64,
}

fn default_op_retention() -> u64 {
    168 // 7 days
}

/// Conflict resolution strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictResolution {
    /// Last write wins
    LastWriteWins,

    /// Manual resolution required
    Manual,

    /// Automatic merge
    Merge,
}

/// Namespace type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NamespaceType {
    /// Private to this machine only
    Standalone,

    /// Shared with master-replica model
    MasterReplica,

    /// Shared with CRDT consensus
    Distributed,
}

/// Access permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permissions {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub admin: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Permissions {
            read: true,
            write: true,
            delete: true,
            admin: false,
        }
    }
}

/// Access rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRule {
    /// Machine ID this rule applies to
    pub machine: Option<String>,

    /// Permissions granted
    #[serde(default)]
    pub permissions: Vec<String>, // ["read", "write", "delete", "admin"]
}

impl AccessRule {
    pub fn to_permissions(&self) -> Permissions {
        let mut perms = Permissions {
            read: false,
            write: false,
            delete: false,
            admin: false,
        };

        for perm in &self.permissions {
            match perm.as_str() {
                "read" => perms.read = true,
                "write" => perms.write = true,
                "delete" => perms.delete = true,
                "admin" => perms.admin = true,
                _ => {}
            }
        }

        perms
    }
}

/// Namespace configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceConfig {
    /// Namespace name
    pub name: String,

    /// Namespace type
    #[serde(rename = "type")]
    pub namespace_type: NamespaceType,

    /// Mount point for this namespace
    pub mount_point: Option<PathBuf>,

    /// Master machine ID (for master-replica namespaces)
    pub master: Option<String>,

    /// Cluster ID (for distributed namespaces)
    pub cluster: Option<String>,

    /// Access control rules
    #[serde(default)]
    pub access: Vec<AccessRule>,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (trace, debug, info, warn, error)
    pub level: String,

    /// Log file path
    pub file: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        LoggingConfig {
            level: "info".to_string(),
            file: None,
        }
    }
}

/// Main configuration structure (v2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigV2 {
    /// Config version
    #[serde(default = "default_version")]
    pub version: u32,

    /// Machine identity
    pub machine: MachineConfig,

    /// Telegram backend configuration
    pub telegram: TelegramConfig,

    /// Encryption configuration
    pub encryption: EncryptionConfig,

    /// Distribution configuration
    pub distribution: DistributionConfig,

    /// Namespaces
    #[serde(default)]
    pub namespaces: Vec<NamespaceConfig>,

    /// Cache configuration
    pub cache: CacheConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Path to the data directory
    #[serde(skip)]
    pub data_dir: PathBuf,
}

fn default_version() -> u32 {
    2
}

/// Main configuration structure (legacy v1 - for backwards compatibility)
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
            .join("tgcryptfs");

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
        // Use compile-time embedded API credentials if available
        let api_id = EMBEDDED_API_ID
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0);
        let api_hash = EMBEDDED_API_HASH
            .map(|s| s.to_string())
            .unwrap_or_default();

        TelegramConfig {
            api_id,
            api_hash,
            phone: None,
            session_file: PathBuf::from("tgcryptfs.session"),
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
            mount_point: PathBuf::from("/mnt/tgcryptfs"),
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
        if let Ok(cache_size) = std::env::var("TGCRYPTFS_CACHE_SIZE") {
            if let Ok(size) = cache_size.trim().parse::<u64>() {
                self.cache.max_size = size;
            }
        }

        // Chunk settings
        if let Ok(chunk_size) = std::env::var("TGCRYPTFS_CHUNK_SIZE") {
            if let Ok(size) = chunk_size.trim().parse::<usize>() {
                self.chunk.chunk_size = size;
            }
        }
    }

    /// Create a new config from environment variables only (for init without existing config)
    ///
    /// API credentials are resolved in this order:
    /// 1. Runtime environment variables (TELEGRAM_APP_ID, TELEGRAM_APP_HASH)
    /// 2. Compile-time embedded credentials (TGCRYPTFS_DEFAULT_API_ID, TGCRYPTFS_DEFAULT_API_HASH)
    ///
    /// If neither are available, an error is returned.
    pub fn from_env() -> Result<Self> {
        let mut config = Config::default();
        config.apply_env_overrides();

        // Check if API credentials are available (either embedded or from env)
        if config.telegram.api_id == 0 {
            let msg = if EMBEDDED_API_ID.is_none() {
                "Telegram API credentials required. Set TELEGRAM_APP_ID environment variable \
                 or get your API credentials from https://my.telegram.org/apps"
            } else {
                "TELEGRAM_APP_ID environment variable is required (embedded default is invalid)"
            };
            return Err(Error::InvalidConfig(msg.to_string()));
        }
        if config.telegram.api_hash.is_empty() {
            let msg = if EMBEDDED_API_HASH.is_none() {
                "Telegram API credentials required. Set TELEGRAM_APP_HASH environment variable \
                 or get your API credentials from https://my.telegram.org/apps"
            } else {
                "TELEGRAM_APP_HASH environment variable is required (embedded default is invalid)"
            };
            return Err(Error::InvalidConfig(msg.to_string()));
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

impl Default for MachineConfig {
    fn default() -> Self {
        MachineConfig {
            id: "auto".to_string(),
            name: hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "tgcryptfs-machine".to_string()),
        }
    }
}

impl Default for DistributionConfig {
    fn default() -> Self {
        DistributionConfig {
            mode: DistributionMode::Standalone,
            cluster_id: None,
            master_replica: None,
            distributed: None,
        }
    }
}

impl Default for ConfigV2 {
    fn default() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tgcryptfs");

        ConfigV2 {
            version: 2,
            machine: MachineConfig::default(),
            telegram: TelegramConfig::default(),
            encryption: EncryptionConfig::default(),
            distribution: DistributionConfig::default(),
            namespaces: vec![],
            cache: CacheConfig {
                max_size: DEFAULT_CACHE_SIZE,
                cache_dir: data_dir.join("cache"),
                prefetch_enabled: true,
                prefetch_count: DEFAULT_PREFETCH_COUNT,
                eviction_policy: EvictionPolicy::Lru,
            },
            logging: LoggingConfig::default(),
            data_dir,
        }
    }
}

impl ConfigV2 {
    /// Load configuration from a file (YAML or JSON), with environment variable substitution
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let content = std::fs::read_to_string(path_ref).map_err(|e| {
            Error::Config(format!("Failed to read config file: {}", e))
        })?;

        // Perform environment variable substitution
        let content = Self::substitute_env_vars(&content);

        // Detect format by extension
        let config: ConfigV2 = if path_ref.extension().and_then(|s| s.to_str()) == Some("yaml")
            || path_ref.extension().and_then(|s| s.to_str()) == Some("yml")
        {
            serde_yaml::from_str(&content).map_err(|e| {
                Error::Config(format!("Failed to parse YAML config: {}", e))
            })?
        } else {
            serde_json::from_str(&content).map_err(|e| {
                Error::Config(format!("Failed to parse JSON config: {}", e))
            })?
        };

        let mut config = config;

        // Set data_dir if not specified
        if config.data_dir == PathBuf::new() {
            config.data_dir = dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("tgcryptfs");
        }

        // Generate machine ID if set to "auto"
        if config.machine.id == "auto" {
            config.machine.id = Uuid::new_v4().to_string();
        }

        config.validate()?;
        Ok(config)
    }

    /// Substitute environment variables in config content
    /// Supports ${VAR_NAME} syntax
    fn substitute_env_vars(content: &str) -> String {
        let mut result = content.to_string();

        // Find all ${VAR_NAME} patterns
        let re = regex::Regex::new(r"\$\{([A-Z_][A-Z0-9_]*)\}").unwrap();

        for cap in re.captures_iter(content) {
            let full_match = &cap[0];
            let var_name = &cap[1];

            if let Ok(value) = std::env::var(var_name) {
                result = result.replace(full_match, &value);
            }
        }

        result
    }

    /// Save configuration to a file (format determined by extension)
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path_ref = path.as_ref();

        let content = if path_ref.extension().and_then(|s| s.to_str()) == Some("yaml")
            || path_ref.extension().and_then(|s| s.to_str()) == Some("yml")
        {
            serde_yaml::to_string(self).map_err(|e| {
                Error::Config(format!("Failed to serialize config to YAML: {}", e))
            })?
        } else {
            serde_json::to_string_pretty(self).map_err(|e| {
                Error::Config(format!("Failed to serialize config to JSON: {}", e))
            })?
        };

        std::fs::write(path_ref, content).map_err(|e| {
            Error::Config(format!("Failed to write config file: {}", e))
        })?;

        Ok(())
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate Telegram config
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

        // Validate distribution mode consistency
        match self.distribution.mode {
            DistributionMode::Standalone => {
                // No special requirements for standalone
            }
            DistributionMode::MasterReplica => {
                if self.distribution.cluster_id.is_none() {
                    return Err(Error::InvalidConfig(
                        "cluster_id is required for master-replica mode".to_string(),
                    ));
                }
                if self.distribution.master_replica.is_none() {
                    return Err(Error::InvalidConfig(
                        "master_replica configuration is required for master-replica mode"
                            .to_string(),
                    ));
                }
            }
            DistributionMode::Distributed => {
                if self.distribution.cluster_id.is_none() {
                    return Err(Error::InvalidConfig(
                        "cluster_id is required for distributed mode".to_string(),
                    ));
                }
                if self.distribution.distributed.is_none() {
                    return Err(Error::InvalidConfig(
                        "distributed configuration is required for distributed mode".to_string(),
                    ));
                }
            }
        }

        // Validate namespace configurations
        for namespace in &self.namespaces {
            match namespace.namespace_type {
                NamespaceType::MasterReplica => {
                    if namespace.master.is_none() {
                        return Err(Error::InvalidConfig(format!(
                            "Namespace '{}': master is required for master-replica type",
                            namespace.name
                        )));
                    }
                }
                NamespaceType::Distributed => {
                    if namespace.cluster.is_none() {
                        return Err(Error::InvalidConfig(format!(
                            "Namespace '{}': cluster is required for distributed type",
                            namespace.name
                        )));
                    }
                }
                NamespaceType::Standalone => {
                    // No special requirements
                }
            }
        }

        Ok(())
    }

    /// Ensure all required directories exist
    pub fn ensure_directories(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.cache.cache_dir)?;

        // Create namespace-specific directories
        for namespace in &self.namespaces {
            let ns_dir = self.data_dir.join("namespaces").join(&namespace.name);
            std::fs::create_dir_all(&ns_dir)?;
        }

        Ok(())
    }

    /// Create a new config from environment variables
    pub fn from_env() -> Result<Self> {
        let mut config = ConfigV2::default();

        // Required variables
        if let Ok(api_id) = std::env::var("TELEGRAM_APP_ID") {
            config.telegram.api_id = api_id.trim().parse().map_err(|_| {
                Error::InvalidConfig("Invalid TELEGRAM_APP_ID".to_string())
            })?;
        } else {
            return Err(Error::InvalidConfig(
                "TELEGRAM_APP_ID environment variable is required".to_string(),
            ));
        }

        if let Ok(api_hash) = std::env::var("TELEGRAM_APP_HASH") {
            config.telegram.api_hash = api_hash.trim().to_string();
        } else {
            return Err(Error::InvalidConfig(
                "TELEGRAM_APP_HASH environment variable is required".to_string(),
            ));
        }

        // Optional variables
        if let Ok(phone) = std::env::var("TELEGRAM_PHONE") {
            config.telegram.phone = Some(phone.trim().to_string());
        }

        if let Ok(machine_name) = std::env::var("TGCRYPTFS_MACHINE_NAME") {
            config.machine.name = machine_name.trim().to_string();
        }

        Ok(config)
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
