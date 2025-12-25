//! TelegramFS - Encrypted filesystem backed by Telegram Saved Messages
//!
//! Usage:
//!   telegramfs mount <mount_point>  - Mount the filesystem
//!   telegramfs init                 - Initialize a new filesystem
//!   telegramfs auth                 - Authenticate with Telegram
//!   telegramfs status               - Show filesystem status
//!   telegramfs snapshot <name>      - Create a snapshot

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use telegramfs::{
    cache::ChunkCache,
    config::Config,
    crypto::{KeyManager, MasterKey},
    fs::TelegramFs,
    metadata::MetadataStore,
    telegram::TelegramBackend,
    Error, Result,
};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser)]
#[command(name = "telegramfs")]
#[command(author = "TelegramFS Contributors")]
#[command(version = "0.1.0")]
#[command(about = "Encrypted filesystem backed by Telegram Saved Messages")]
struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "~/.config/telegramfs/config.json")]
    config: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new TelegramFS
    Init {
        /// Telegram API ID (from my.telegram.org)
        #[arg(long)]
        api_id: i32,

        /// Telegram API hash
        #[arg(long)]
        api_hash: String,

        /// Phone number for authentication
        #[arg(long)]
        phone: Option<String>,
    },

    /// Authenticate with Telegram
    Auth {
        /// Phone number
        #[arg(long)]
        phone: String,
    },

    /// Mount the filesystem
    Mount {
        /// Mount point directory
        mount_point: PathBuf,

        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,

        /// Allow other users to access the mount
        #[arg(long)]
        allow_other: bool,
    },

    /// Unmount the filesystem
    Unmount {
        /// Mount point to unmount
        mount_point: PathBuf,
    },

    /// Show filesystem status
    Status,

    /// Create a snapshot
    Snapshot {
        /// Snapshot name
        name: String,

        /// Optional description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// List snapshots
    Snapshots,

    /// Restore from a snapshot
    Restore {
        /// Snapshot name or ID
        snapshot: String,
    },

    /// Show cache statistics
    Cache {
        /// Clear the cache
        #[arg(long)]
        clear: bool,
    },

    /// Sync local state with Telegram
    Sync {
        /// Force full sync
        #[arg(long)]
        full: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    // Setup logging
    let log_level = if cli.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .with_target(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set subscriber");

    // Expand ~ in config path
    let config_path = expand_tilde(&cli.config);

    // Run the command
    if let Err(e) = run_command(cli.command, &config_path) {
        error!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_command(command: Commands, config_path: &PathBuf) -> Result<()> {
    match command {
        Commands::Init {
            api_id,
            api_hash,
            phone,
        } => cmd_init(config_path, api_id, api_hash, phone),

        Commands::Auth { phone } => cmd_auth(config_path, &phone),

        Commands::Mount {
            mount_point,
            foreground,
            allow_other,
        } => cmd_mount(config_path, &mount_point, foreground, allow_other),

        Commands::Unmount { mount_point } => cmd_unmount(&mount_point),

        Commands::Status => cmd_status(config_path),

        Commands::Snapshot { name, description } => cmd_snapshot(config_path, &name, description),

        Commands::Snapshots => cmd_list_snapshots(config_path),

        Commands::Restore { snapshot } => cmd_restore(config_path, &snapshot),

        Commands::Cache { clear } => cmd_cache(config_path, clear),

        Commands::Sync { full } => cmd_sync(config_path, full),
    }
}

fn cmd_init(
    config_path: &PathBuf,
    api_id: i32,
    api_hash: String,
    phone: Option<String>,
) -> Result<()> {
    info!("Initializing TelegramFS...");

    // Create default config
    let mut config = Config::default();
    config.telegram.api_id = api_id;
    config.telegram.api_hash = api_hash;
    config.telegram.phone = phone;

    // Ensure config directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Save config
    config.save(config_path)?;

    // Create data directories
    config.ensure_directories()?;

    info!("Configuration saved to {:?}", config_path);
    info!("Data directory: {:?}", config.data_dir);
    info!("");
    info!("Next steps:");
    info!("  1. Run 'telegramfs auth --phone <your_phone>' to authenticate");
    info!("  2. Run 'telegramfs mount <mount_point>' to mount the filesystem");

    Ok(())
}

fn cmd_auth(config_path: &PathBuf, phone: &str) -> Result<()> {
    let config = Config::load(config_path)?;

    info!("Authenticating with Telegram...");

    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error::Internal(e.to_string()))?;

    runtime.block_on(async {
        let backend = TelegramBackend::new(config.telegram.clone());
        backend.connect().await?;

        if backend.is_authorized().await? {
            info!("Already authenticated!");
            return Ok(());
        }

        // Request code
        backend.request_login_code(phone).await?;
        info!("Login code sent to {}", phone);

        // Get code from user
        print!("Enter the code you received: ");
        use std::io::Write;
        std::io::stdout().flush()?;

        let mut code = String::new();
        std::io::stdin().read_line(&mut code)?;
        let code = code.trim();

        // Sign in
        backend.sign_in(phone, code).await?;
        info!("Successfully authenticated!");

        backend.disconnect().await;
        Ok(())
    })
}

fn cmd_mount(
    config_path: &PathBuf,
    mount_point: &PathBuf,
    foreground: bool,
    allow_other: bool,
) -> Result<()> {
    let mut config = Config::load(config_path)?;
    config.mount.mount_point = mount_point.clone();
    config.mount.allow_other = allow_other;

    info!("Starting TelegramFS...");

    // Get password for key derivation
    let password = rpassword::prompt_password("Enter encryption password: ")
        .map_err(|e| Error::Internal(e.to_string()))?;

    // Derive master key
    let master_key = MasterKey::from_password(password.as_bytes(), &config.encryption)?;
    let key_manager = KeyManager::new(master_key)?;

    // Update config with salt if new
    if config.encryption.salt.is_empty() {
        config.encryption.salt = key_manager.salt().to_vec();
        config.save(config_path)?;
    }

    // Create metadata store
    let metadata_path = config.data_dir.join("metadata.db");
    let metadata = MetadataStore::open(&metadata_path, *key_manager.metadata_key())?;

    // Create Telegram backend
    let telegram = TelegramBackend::new(config.telegram.clone());

    // Connect to Telegram
    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error::Internal(e.to_string()))?;
    runtime.block_on(async {
        telegram.connect().await?;
        if !telegram.is_authorized().await? {
            return Err(Error::TelegramAuthRequired);
        }
        Ok::<_, Error>(())
    })?;

    // Create cache
    let cache = ChunkCache::new(&config.cache)?;

    // Create filesystem
    let fs = TelegramFs::new(config.clone(), key_manager, metadata, telegram, cache)?;

    // Ensure mount point exists
    std::fs::create_dir_all(mount_point)?;

    info!("Mounting at {:?}", mount_point);

    // Build mount options
    let mut options = vec![
        fuser::MountOption::FSName("telegramfs".to_string()),
        fuser::MountOption::AutoUnmount,
    ];

    if allow_other {
        options.push(fuser::MountOption::AllowOther);
    }

    if foreground {
        // Mount in foreground
        fuser::mount2(fs, mount_point, &options).map_err(|e| Error::Internal(e.to_string()))?;
    } else {
        // Daemonize
        info!("Daemonizing... Use 'telegramfs unmount {:?}' to unmount", mount_point);

        // For proper daemonization, you'd use a crate like `daemonize`
        // For now, just run in foreground
        fuser::mount2(fs, mount_point, &options).map_err(|e| Error::Internal(e.to_string()))?;
    }

    Ok(())
}

fn cmd_unmount(mount_point: &PathBuf) -> Result<()> {
    info!("Unmounting {:?}...", mount_point);

    // Use fusermount/umount
    #[cfg(target_os = "linux")]
    let output = std::process::Command::new("fusermount")
        .arg("-u")
        .arg(mount_point)
        .output()?;

    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("umount")
        .arg(mount_point)
        .output()?;

    if output.status.success() {
        info!("Unmounted successfully");
        Ok(())
    } else {
        Err(Error::Internal(format!(
            "Failed to unmount: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn cmd_status(config_path: &PathBuf) -> Result<()> {
    let config = Config::load(config_path)?;

    println!("TelegramFS Status");
    println!("=================");
    println!();
    println!("Configuration: {:?}", config_path);
    println!("Data directory: {:?}", config.data_dir);
    println!("Cache directory: {:?}", config.cache.cache_dir);
    println!("Cache max size: {} MB", config.cache.max_size / 1024 / 1024);
    println!("Chunk size: {} MB", config.chunk.chunk_size / 1024 / 1024);
    println!("Compression: {}", if config.chunk.compression_enabled { "enabled" } else { "disabled" });
    println!("Deduplication: {}", if config.chunk.dedup_enabled { "enabled" } else { "disabled" });
    println!("Versioning: {}", if config.versioning.enabled { "enabled" } else { "disabled" });

    // Check Telegram connection
    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error::Internal(e.to_string()))?;
    runtime.block_on(async {
        let backend = TelegramBackend::new(config.telegram.clone());
        match backend.connect().await {
            Ok(_) => {
                if backend.is_authorized().await.unwrap_or(false) {
                    println!("Telegram: connected and authorized");
                } else {
                    println!("Telegram: connected but NOT authorized (run 'telegramfs auth')");
                }
                backend.disconnect().await;
            }
            Err(e) => {
                println!("Telegram: connection failed - {}", e);
            }
        }
        Ok::<_, Error>(())
    })?;

    Ok(())
}

fn cmd_snapshot(config_path: &PathBuf, name: &str, description: Option<String>) -> Result<()> {
    info!("Creating snapshot '{}'...", name);

    // This would require loading the full filesystem state
    // Simplified version just logs the intent
    println!("Snapshot creation not yet fully implemented");
    println!("Would create snapshot: {} - {:?}", name, description);

    Ok(())
}

fn cmd_list_snapshots(config_path: &PathBuf) -> Result<()> {
    println!("Snapshots:");
    println!("==========");
    println!("(Snapshot listing not yet fully implemented)");
    Ok(())
}

fn cmd_restore(config_path: &PathBuf, snapshot: &str) -> Result<()> {
    info!("Restoring from snapshot '{}'...", snapshot);
    println!("Snapshot restoration not yet fully implemented");
    Ok(())
}

fn cmd_cache(config_path: &PathBuf, clear: bool) -> Result<()> {
    let config = Config::load(config_path)?;

    if clear {
        info!("Clearing cache...");
        let cache = ChunkCache::new(&config.cache)?;
        cache.clear()?;
        info!("Cache cleared");
    } else {
        let cache = ChunkCache::new(&config.cache)?;
        let stats = cache.stats();

        println!("Cache Statistics");
        println!("================");
        println!("Size: {} / {} MB ({:.1}%)",
            stats.current_size / 1024 / 1024,
            stats.max_size / 1024 / 1024,
            stats.utilization()
        );
        println!("Chunks cached: {}", stats.chunk_count);
        println!("Prefetch queue: {}", stats.prefetch_queue_len);
    }

    Ok(())
}

fn cmd_sync(config_path: &PathBuf, full: bool) -> Result<()> {
    info!("Syncing with Telegram...");

    if full {
        info!("Performing full sync...");
    }

    println!("Sync not yet fully implemented");
    Ok(())
}

/// Expand ~ to home directory
fn expand_tilde(path: &PathBuf) -> PathBuf {
    if path.starts_with("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(path.strip_prefix("~").unwrap());
        }
    }
    path.clone()
}
