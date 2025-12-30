//! tgcryptfs - Encrypted cloud-backed filesystem
//!
//! Usage:
//!   tgcryptfs mount <mount_point>  - Mount the filesystem
//!   tgcryptfs init                 - Initialize a new filesystem
//!   tgcryptfs auth                 - Authenticate with the cloud backend
//!   tgcryptfs status               - Show filesystem status
//!   tgcryptfs snapshot <name>      - Create a snapshot

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tgcryptfs::{
    cache::ChunkCache,
    config::Config,
    crypto::{KeyManager, MasterKey},
    fs::{overlay::{OverlayConfig, OverlayFs}, TgCryptFs},
    metadata::MetadataStore,
    telegram::TelegramBackend,
    Error, Result,
};
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser)]
#[command(name = "tgcryptfs")]
#[command(author = "tgcryptfs Contributors")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Encrypted cloud-backed filesystem")]
struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "~/.config/tgcryptfs/config.json")]
    config: PathBuf,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new tgcryptfs
    Init {
        /// API ID (from my.telegram.org)
        #[arg(long)]
        api_id: i32,

        /// API hash
        #[arg(long)]
        api_hash: String,

        /// Phone number for authentication
        #[arg(long)]
        phone: Option<String>,
    },

    /// Authenticate with the cloud backend
    Auth {
        /// Phone number
        #[arg(long)]
        phone: String,

        /// Login code (if not provided, will prompt interactively)
        #[arg(long)]
        code: Option<String>,

        /// 2FA password (if required)
        #[arg(long)]
        password: Option<String>,
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

        /// Read encryption password from file
        #[arg(long)]
        password_file: Option<PathBuf>,

        /// Enable overlay mode (lower layer read-only, writes go to upper layer)
        #[arg(long)]
        overlay: bool,

        /// Lower layer path for overlay mode (defaults to home directory)
        #[arg(long)]
        lower_path: Option<PathBuf>,
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

    /// Machine management
    #[command(subcommand)]
    Machine(MachineCommands),

    /// Namespace management
    #[command(subcommand)]
    Namespace(NamespaceCommands),

    /// Cluster management
    #[command(subcommand)]
    Cluster(ClusterCommands),

    /// RAID/Erasure coding management
    #[command(subcommand)]
    Raid(RaidCommands),

    /// Migrate HKDF from telegramfs-* to tgcryptfs-*
    Migrate {
        /// Read encryption password from file
        #[arg(long)]
        password_file: Option<PathBuf>,

        /// Perform a dry run (don't actually modify data)
        #[arg(long)]
        dry_run: bool,

        /// Force migration even if already migrated
        #[arg(long)]
        force: bool,
    },

    /// Time Machine backup management
    #[command(subcommand)]
    Timemachine(TimemachineCommands),
}

#[derive(Subcommand)]
enum MachineCommands {
    /// Initialize machine identity
    Init {
        /// Machine name
        #[arg(long)]
        name: Option<String>,
    },

    /// Show machine identity
    Show,
}

#[derive(Subcommand)]
enum NamespaceCommands {
    /// Create a new namespace
    Create {
        /// Namespace name
        name: String,

        /// Namespace type
        #[arg(long, value_parser = ["standalone", "master-replica", "distributed"])]
        r#type: String,

        /// Mount point for this namespace
        #[arg(long)]
        mount_point: Option<PathBuf>,

        /// Master machine ID (for master-replica)
        #[arg(long)]
        master: Option<String>,

        /// Cluster ID (for distributed)
        #[arg(long)]
        cluster: Option<String>,
    },

    /// List all namespaces
    List,
}

#[derive(Subcommand)]
enum ClusterCommands {
    /// Create a new cluster
    Create {
        /// Cluster ID
        cluster_id: String,
    },

    /// Join an existing cluster
    Join {
        /// Cluster ID to join
        cluster_id: String,

        /// Role in the cluster
        #[arg(long, value_parser = ["master", "replica", "node"])]
        role: String,
    },

    /// Show cluster status
    Status,
}

#[derive(Subcommand)]
enum RaidCommands {
    /// Show RAID array status
    Status,

    /// Rebuild data for a failed account
    Rebuild {
        /// Account ID to rebuild (0-indexed)
        account_id: u8,
    },

    /// Verify all stripes (scrub operation)
    Scrub {
        /// Fix any issues found
        #[arg(long)]
        repair: bool,
    },

    /// Add a new account to the pool
    AddAccount {
        /// Telegram API ID
        #[arg(long)]
        api_id: i32,

        /// Telegram API hash
        #[arg(long)]
        api_hash: String,

        /// Session file path
        #[arg(long)]
        session_file: PathBuf,

        /// Phone number (optional, can prompt later)
        #[arg(long)]
        phone: Option<String>,
    },

    /// Migrate existing single-account data to erasure-coded multi-account
    MigrateToErasure {
        /// Perform a dry run (don't modify data)
        #[arg(long)]
        dry_run: bool,

        /// Delete old single-account messages after successful migration
        #[arg(long)]
        delete_old: bool,
    },
}

#[derive(Subcommand)]
enum TimemachineCommands {
    /// Initialize Time Machine backup volume
    Init {
        /// Size of the backup volume (e.g., 500g, 1t)
        #[arg(long, default_value = "500g")]
        size: String,

        /// Volume name
        #[arg(long, default_value = "TGCryptFS-TM")]
        name: String,
    },

    /// Mount Time Machine backup volume
    Mount {
        /// Mount point for the Time Machine volume
        mount_point: Option<PathBuf>,
    },

    /// Unmount Time Machine backup volume
    Unmount,

    /// Show Time Machine backup status
    Status,

    /// Sync local sparsebundle to cloud
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

        Commands::Auth { phone, code, password } => cmd_auth(config_path, &phone, code, password),

        Commands::Mount {
            mount_point,
            foreground,
            allow_other,
            password_file,
            overlay,
            lower_path,
        } => cmd_mount(config_path, &mount_point, foreground, allow_other, password_file, overlay, lower_path),

        Commands::Unmount { mount_point } => cmd_unmount(&mount_point),

        Commands::Status => cmd_status(config_path),

        Commands::Snapshot { name, description } => cmd_snapshot(config_path, &name, description),

        Commands::Snapshots => cmd_list_snapshots(config_path),

        Commands::Restore { snapshot } => cmd_restore(config_path, &snapshot),

        Commands::Cache { clear } => cmd_cache(config_path, clear),

        Commands::Sync { full } => cmd_sync(config_path, full),

        Commands::Machine(machine_cmd) => run_machine_command(machine_cmd, config_path),

        Commands::Namespace(namespace_cmd) => run_namespace_command(namespace_cmd, config_path),

        Commands::Cluster(cluster_cmd) => run_cluster_command(cluster_cmd, config_path),

        Commands::Raid(raid_cmd) => run_raid_command(raid_cmd, config_path),

        Commands::Migrate {
            password_file,
            dry_run,
            force,
        } => cmd_migrate(config_path, password_file, dry_run, force),

        Commands::Timemachine(tm_cmd) => run_timemachine_command(tm_cmd, config_path),
    }
}

fn run_machine_command(command: MachineCommands, config_path: &PathBuf) -> Result<()> {
    match command {
        MachineCommands::Init { name } => cmd_machine_init(config_path, name),
        MachineCommands::Show => cmd_machine_show(config_path),
    }
}

fn run_namespace_command(command: NamespaceCommands, config_path: &PathBuf) -> Result<()> {
    match command {
        NamespaceCommands::Create {
            name,
            r#type,
            mount_point,
            master,
            cluster,
        } => cmd_namespace_create(config_path, name, r#type, mount_point, master, cluster),
        NamespaceCommands::List => cmd_namespace_list(config_path),
    }
}

fn run_cluster_command(command: ClusterCommands, config_path: &PathBuf) -> Result<()> {
    match command {
        ClusterCommands::Create { cluster_id } => cmd_cluster_create(config_path, cluster_id),
        ClusterCommands::Join { cluster_id, role } => cmd_cluster_join(config_path, cluster_id, role),
        ClusterCommands::Status => cmd_cluster_status(config_path),
    }
}

fn run_raid_command(command: RaidCommands, config_path: &PathBuf) -> Result<()> {
    match command {
        RaidCommands::Status => cmd_raid_status(config_path),
        RaidCommands::Rebuild { account_id } => cmd_raid_rebuild(config_path, account_id),
        RaidCommands::Scrub { repair } => cmd_raid_scrub(config_path, repair),
        RaidCommands::AddAccount {
            api_id,
            api_hash,
            session_file,
            phone,
        } => cmd_raid_add_account(config_path, api_id, api_hash, session_file, phone),
        RaidCommands::MigrateToErasure { dry_run, delete_old } => {
            cmd_raid_migrate(config_path, dry_run, delete_old)
        }
    }
}

fn cmd_init(
    config_path: &PathBuf,
    api_id: i32,
    api_hash: String,
    phone: Option<String>,
) -> Result<()> {
    info!("Initializing tgcryptfs...");

    // Create default config
    let mut config = Config::default();

    // Use provided args or fall back to environment variables
    config.telegram.api_id = if api_id != 0 {
        api_id
    } else if let Ok(env_id) = std::env::var("TELEGRAM_APP_ID") {
        env_id.parse().unwrap_or(0)
    } else {
        0
    };

    config.telegram.api_hash = if !api_hash.is_empty() {
        api_hash
    } else {
        std::env::var("TELEGRAM_APP_HASH").unwrap_or_default()
    };

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
    info!("  1. Run 'tgcryptfs auth --phone <your_phone>' to authenticate");
    info!("  2. Run 'tgcryptfs mount <mount_point>' to mount the filesystem");

    Ok(())
}

fn cmd_auth(config_path: &PathBuf, phone: &str, code_opt: Option<String>, password_opt: Option<String>) -> Result<()> {
    let config = Config::load(config_path)?;

    info!("Authenticating with cloud backend...");

    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error::Internal(e.to_string()))?;

    runtime.block_on(async {
        let backend = TelegramBackend::new(config.telegram.clone());
        backend.connect().await?;

        if backend.is_authorized().await? {
            info!("Already authenticated!");
            return Ok(());
        }

        // Request code
        let login_token = backend.request_login_code(phone).await?;
        info!("Login code sent to {}", phone);

        // Get code from user (or use provided code)
        let code = if let Some(c) = code_opt {
            c
        } else {
            print!("Enter the code you received: ");
            use std::io::Write;
            std::io::stdout().flush()?;

            let mut code = String::new();
            std::io::stdin().read_line(&mut code)?;
            code.trim().to_string()
        };

        // Sign in
        match backend.sign_in(&login_token, &code).await? {
            Some(password_token) => {
                // 2FA required
                let hint = password_token.hint().unwrap_or("none");
                println!("2FA required (hint: {})", hint);
                let password = if let Some(p) = password_opt {
                    p
                } else {
                    rpassword::prompt_password("Enter your 2FA password: ")
                        .map_err(|e| Error::Internal(e.to_string()))?
                };
                backend.check_password(password_token, &password).await?;
            }
            None => {}
        }
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
    password_file: Option<PathBuf>,
    overlay: bool,
    lower_path: Option<PathBuf>,
) -> Result<()> {
    let mut config = Config::load(config_path)?;
    config.mount.mount_point = mount_point.clone();
    config.mount.allow_other = allow_other;

    // Build mount options
    let mut options = vec![
        fuser::MountOption::FSName("tgcryptfs".to_string()),
        fuser::MountOption::AutoUnmount,
    ];

    if allow_other {
        options.push(fuser::MountOption::AllowOther);
    }

    // Ensure mount point exists
    std::fs::create_dir_all(mount_point)?;

    if overlay {
        // Overlay mode: local lower layer + local upper layer
        info!("Starting tgcryptfs in OVERLAY mode...");

        let lower = lower_path.unwrap_or_else(|| {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
        });

        let overlay_config = OverlayConfig::with_lower_path(lower.clone());

        info!("Lower layer (read-only): {:?}", lower);
        info!("Upper layer (writable): {:?}", overlay_config.upper_path);
        info!("Whiteout DB: {:?}", overlay_config.whiteout_db_path);

        // Create upper layer directory
        std::fs::create_dir_all(&overlay_config.upper_path)?;

        // Create overlay filesystem
        let fs = OverlayFs::new(overlay_config)
            .map_err(|e| Error::Internal(format!("Failed to create overlay: {}", e)))?;

        info!("Mounting overlay at {:?}", mount_point);

        if foreground {
            fuser::mount2(fs, mount_point, &options).map_err(|e| Error::Internal(e.to_string()))?;
        } else {
            info!("Daemonizing... Use 'tgcryptfs unmount {:?}' to unmount", mount_point);
            fuser::mount2(fs, mount_point, &options).map_err(|e| Error::Internal(e.to_string()))?;
        }
    } else {
        // Standard mode: Telegram-backed filesystem
        info!("Starting tgcryptfs...");

        // Get password for key derivation
        let password = if let Some(path) = password_file {
            std::fs::read_to_string(&path)
                .map_err(|e| Error::Internal(format!("Failed to read password file: {}", e)))?
                .trim()
                .to_string()
        } else {
            rpassword::prompt_password("Enter encryption password: ")
                .map_err(|e| Error::Internal(e.to_string()))?
        };

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
        let fs = TgCryptFs::new(config.clone(), key_manager, metadata, telegram, cache)?;

        info!("Mounting at {:?}", mount_point);

        if foreground {
            fuser::mount2(fs, mount_point, &options).map_err(|e| Error::Internal(e.to_string()))?;
        } else {
            info!("Daemonizing... Use 'tgcryptfs unmount {:?}' to unmount", mount_point);
            fuser::mount2(fs, mount_point, &options).map_err(|e| Error::Internal(e.to_string()))?;
        }
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

    println!("tgcryptfs Status");
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

    // Check cloud backend connection
    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error::Internal(e.to_string()))?;
    runtime.block_on(async {
        let backend = TelegramBackend::new(config.telegram.clone());
        match backend.connect().await {
            Ok(_) => {
                if backend.is_authorized().await.unwrap_or(false) {
                    println!("Cloud backend: connected and authorized");
                } else {
                    println!("Cloud backend: connected but NOT authorized (run 'tgcryptfs auth')");
                }
                backend.disconnect().await;
            }
            Err(e) => {
                println!("Cloud backend: connection failed - {}", e);
            }
        }
        Ok::<_, Error>(())
    })?;

    Ok(())
}

fn cmd_snapshot(_config_path: &PathBuf, name: &str, description: Option<String>) -> Result<()> {
    info!("Creating snapshot '{}'...", name);

    // This would require loading the full filesystem state
    // Simplified version just logs the intent
    println!("Snapshot creation not yet fully implemented");
    println!("Would create snapshot: {} - {:?}", name, description);

    Ok(())
}

fn cmd_list_snapshots(_config_path: &PathBuf) -> Result<()> {
    println!("Snapshots:");
    println!("==========");
    println!("(Snapshot listing not yet fully implemented)");
    Ok(())
}

fn cmd_restore(_config_path: &PathBuf, snapshot: &str) -> Result<()> {
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

fn cmd_sync(_config_path: &PathBuf, full: bool) -> Result<()> {
    info!("Syncing with cloud backend...");

    if full {
        info!("Performing full sync...");
    }

    println!("Sync not yet fully implemented");
    Ok(())
}

fn cmd_machine_init(config_path: &PathBuf, name: Option<String>) -> Result<()> {
    use tgcryptfs::config::ConfigV2;
    use uuid::Uuid;

    info!("Initializing machine identity...");

    // Load or create config
    let mut config = if config_path.exists() {
        ConfigV2::load(config_path)?
    } else {
        ConfigV2::from_env()?
    };

    // Set machine name if provided
    if let Some(name) = name {
        config.machine.name = name;
    }

    // Generate machine ID if not already set
    if config.machine.id == "auto" || config.machine.id.is_empty() {
        config.machine.id = Uuid::new_v4().to_string();
    }

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    config.save(config_path)?;

    println!("Machine initialized:");
    println!("  ID: {}", config.machine.id);
    println!("  Name: {}", config.machine.name);
    println!("  Config: {:?}", config_path);

    Ok(())
}

fn cmd_machine_show(config_path: &PathBuf) -> Result<()> {
    use tgcryptfs::config::ConfigV2;

    let config = ConfigV2::load(config_path)?;

    println!("Machine Identity");
    println!("================");
    println!("ID: {}", config.machine.id);
    println!("Name: {}", config.machine.name);
    println!();
    println!("Distribution Mode: {:?}", config.distribution.mode);
    if let Some(cluster_id) = &config.distribution.cluster_id {
        println!("Cluster ID: {}", cluster_id);
    }

    Ok(())
}

fn cmd_namespace_create(
    config_path: &PathBuf,
    name: String,
    ns_type: String,
    mount_point: Option<PathBuf>,
    master: Option<String>,
    cluster: Option<String>,
) -> Result<()> {
    use tgcryptfs::config::{ConfigV2, NamespaceConfig, NamespaceType};

    info!("Creating namespace '{}'...", name);

    let mut config = ConfigV2::load(config_path)?;

    // Check if namespace already exists
    if config.namespaces.iter().any(|ns| ns.name == name) {
        return Err(Error::InvalidConfig(format!(
            "Namespace '{}' already exists",
            name
        )));
    }

    // Parse namespace type
    let namespace_type = match ns_type.as_str() {
        "standalone" => NamespaceType::Standalone,
        "master-replica" => {
            if master.is_none() {
                return Err(Error::InvalidConfig(
                    "Master-replica namespaces require --master".to_string(),
                ));
            }
            NamespaceType::MasterReplica
        }
        "distributed" => {
            if cluster.is_none() {
                return Err(Error::InvalidConfig(
                    "Distributed namespaces require --cluster".to_string(),
                ));
            }
            NamespaceType::Distributed
        }
        _ => {
            return Err(Error::InvalidConfig(format!(
                "Invalid namespace type: {}",
                ns_type
            )))
        }
    };

    let namespace = NamespaceConfig {
        name: name.clone(),
        namespace_type,
        mount_point,
        master,
        cluster,
        access: vec![],
    };

    config.namespaces.push(namespace);
    config.save(config_path)?;

    println!("Namespace '{}' created successfully", name);
    println!("  Type: {}", ns_type);

    Ok(())
}

fn cmd_namespace_list(config_path: &PathBuf) -> Result<()> {
    use tgcryptfs::config::ConfigV2;

    let config = ConfigV2::load(config_path)?;

    println!("Namespaces");
    println!("==========");

    if config.namespaces.is_empty() {
        println!("No namespaces configured");
        return Ok(());
    }

    for ns in &config.namespaces {
        println!();
        println!("Name: {}", ns.name);
        println!("  Type: {:?}", ns.namespace_type);
        if let Some(mount) = &ns.mount_point {
            println!("  Mount: {:?}", mount);
        }
        if let Some(master) = &ns.master {
            println!("  Master: {}", master);
        }
        if let Some(cluster) = &ns.cluster {
            println!("  Cluster: {}", cluster);
        }
    }

    Ok(())
}

fn cmd_cluster_create(config_path: &PathBuf, cluster_id: String) -> Result<()> {
    use tgcryptfs::config::{ConfigV2, DistributedConfig, DistributionMode, ConflictResolution};

    info!("Creating cluster '{}'...", cluster_id);

    let mut config = ConfigV2::load(config_path)?;

    // Update distribution config
    config.distribution.mode = DistributionMode::Distributed;
    config.distribution.cluster_id = Some(cluster_id.clone());
    config.distribution.distributed = Some(DistributedConfig {
        sync_interval_ms: 1000,
        conflict_resolution: ConflictResolution::LastWriteWins,
        operation_log_retention_hours: 168,
    });

    config.save(config_path)?;

    println!("Cluster '{}' created successfully", cluster_id);
    println!("  Mode: Distributed");
    println!("  Machine ID: {}", config.machine.id);

    Ok(())
}

fn cmd_cluster_join(config_path: &PathBuf, cluster_id: String, role: String) -> Result<()> {
    use tgcryptfs::config::{
        ConfigV2, DistributedConfig, DistributionMode, MasterReplicaConfig, ReplicaRole,
        ConflictResolution,
    };

    info!("Joining cluster '{}'...", cluster_id);

    let mut config = ConfigV2::load(config_path)?;

    match role.as_str() {
        "master" | "replica" => {
            config.distribution.mode = DistributionMode::MasterReplica;
            config.distribution.cluster_id = Some(cluster_id.clone());

            let replica_role = if role == "master" {
                ReplicaRole::Master
            } else {
                ReplicaRole::Replica
            };

            config.distribution.master_replica = Some(MasterReplicaConfig {
                role: replica_role,
                master_id: if role == "master" {
                    config.machine.id.clone()
                } else {
                    String::new() // Must be set manually for replicas
                },
                sync_interval_secs: 60,
                snapshot_retention: 10,
            });

            println!("Joined cluster '{}' as {}", cluster_id, role);
            if role == "replica" {
                println!("NOTE: You must set the master_id in the config file");
            }
        }
        "node" => {
            config.distribution.mode = DistributionMode::Distributed;
            config.distribution.cluster_id = Some(cluster_id.clone());
            config.distribution.distributed = Some(DistributedConfig {
                sync_interval_ms: 1000,
                conflict_resolution: ConflictResolution::LastWriteWins,
                operation_log_retention_hours: 168,
            });

            println!("Joined cluster '{}' as distributed node", cluster_id);
        }
        _ => {
            return Err(Error::InvalidConfig(format!("Invalid role: {}", role)));
        }
    }

    config.save(config_path)?;
    println!("  Machine ID: {}", config.machine.id);

    Ok(())
}

fn cmd_cluster_status(config_path: &PathBuf) -> Result<()> {
    use tgcryptfs::config::ConfigV2;

    let config = ConfigV2::load(config_path)?;

    println!("Cluster Status");
    println!("==============");
    println!();
    println!("Machine: {} ({})", config.machine.name, config.machine.id);
    println!("Mode: {:?}", config.distribution.mode);

    if let Some(cluster_id) = &config.distribution.cluster_id {
        println!("Cluster: {}", cluster_id);
    } else {
        println!("Cluster: None (standalone mode)");
        return Ok(());
    }

    if let Some(mr_config) = &config.distribution.master_replica {
        println!();
        println!("Master-Replica Configuration:");
        println!("  Role: {:?}", mr_config.role);
        println!("  Master ID: {}", mr_config.master_id);
        println!("  Sync Interval: {}s", mr_config.sync_interval_secs);
        println!("  Snapshot Retention: {}", mr_config.snapshot_retention);
    }

    if let Some(dist_config) = &config.distribution.distributed {
        println!();
        println!("Distributed Configuration:");
        println!("  Sync Interval: {}ms", dist_config.sync_interval_ms);
        println!(
            "  Conflict Resolution: {:?}",
            dist_config.conflict_resolution
        );
        println!(
            "  Op Log Retention: {}h",
            dist_config.operation_log_retention_hours
        );
    }

    Ok(())
}

fn cmd_migrate(
    config_path: &PathBuf,
    password_file: Option<PathBuf>,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    use tgcryptfs::migration::{detect_hkdf_version, migrate_metadata_db, HkdfMigration, HkdfVersion};

    let config = Config::load(config_path)?;

    info!("Starting HKDF migration...");
    if dry_run {
        info!("DRY RUN MODE - no changes will be made");
    }

    // Get password
    let password = if let Some(path) = password_file {
        std::fs::read_to_string(&path)
            .map_err(|e| Error::Internal(format!("Failed to read password file: {}", e)))?
            .trim()
            .to_string()
    } else {
        rpassword::prompt_password("Enter encryption password: ")
            .map_err(|e| Error::Internal(e.to_string()))?
    };

    // Derive master key
    let master_key = MasterKey::from_password(password.as_bytes(), &config.encryption)?;

    // Get salt from config
    if config.encryption.salt.is_empty() {
        return Err(Error::InvalidConfig("No salt in configuration - filesystem not initialized".to_string()));
    }

    let salt_bytes: [u8; 32] = config.encryption.salt.as_slice()
        .try_into()
        .map_err(|_| Error::InvalidConfig("Invalid salt length".to_string()))?;

    // Create migration context
    let migration = HkdfMigration::new(master_key.key(), &salt_bytes)?;

    // Open metadata database to check current version
    let metadata_path = config.data_dir.join("metadata.db");

    if !metadata_path.exists() {
        return Err(Error::Internal("Metadata database not found - nothing to migrate".to_string()));
    }

    // Check current HKDF version by sampling a metadata entry
    let db = sled::open(&metadata_path)?;

    // Find the inodes tree
    let inodes_tree = db.open_tree("inodes")?;

    // Get a sample entry to detect version
    if let Some(first) = inodes_tree.first()?
    {
        let (_, value) = first;
        let version = detect_hkdf_version(
            &value,
            migration.old_metadata_key(),
            migration.new_metadata_key(),
        );

        println!("Current HKDF version: {}", version);

        match version {
            HkdfVersion::New => {
                if !force {
                    println!("Data is already using new HKDF strings. No migration needed.");
                    println!("Use --force to re-migrate anyway.");
                    return Ok(());
                }
                println!("Force mode: re-migrating data...");
            }
            HkdfVersion::Old => {
                println!("Data is using old HKDF strings. Migration required.");
            }
            HkdfVersion::Unknown => {
                return Err(Error::Decryption(
                    "Cannot decrypt data with either old or new keys. Wrong password?".to_string()
                ));
            }
        }
    } else {
        println!("No inode entries found in database.");
        return Ok(());
    }

    // Close the db before migration
    drop(inodes_tree);
    drop(db);

    if dry_run {
        println!("\nDry run complete. Would migrate:");
        println!("  - Metadata database at {:?}", metadata_path);
        println!("\nRun without --dry-run to perform the actual migration.");
        return Ok(());
    }

    // Perform metadata migration
    println!("\nMigrating metadata database...");
    let stats = migrate_metadata_db(&metadata_path, &migration)?;

    println!("\nMigration complete!");
    println!("  Entries migrated: {}", stats.entries_migrated);
    println!("  Entries failed: {}", stats.entries_failed);

    if stats.entries_failed > 0 {
        warn!("Some entries failed to migrate. Check logs for details.");
    }

    println!("\nIMPORTANT: After migration, you must:");
    println!("  1. Update the HKDF strings in src/crypto/keys.rs to use 'tgcryptfs-*'");
    println!("  2. Rebuild and reinstall tgcryptfs");
    println!("  3. Test mounting the filesystem");

    Ok(())
}

fn cmd_raid_status(config_path: &PathBuf) -> Result<()> {
    use tgcryptfs::config::ConfigV2;
    use tgcryptfs::raid::{AccountPool, ArrayStatus};

    let config = ConfigV2::load(config_path)?;

    // Check if erasure coding is configured
    let pool_config = config.pool.ok_or_else(|| {
        Error::InvalidConfig("No pool configuration found. Run 'tgcryptfs raid add-account' first.".to_string())
    })?;

    if !pool_config.erasure.enabled {
        println!("Erasure coding is disabled.");
        return Ok(());
    }

    println!("RAID Array Status");
    println!("=================");
    println!();

    println!("Configuration:");
    println!("  Data chunks (K): {}", pool_config.erasure.data_chunks);
    println!("  Total chunks (N): {}", pool_config.erasure.total_chunks);
    println!("  Fault tolerance: {} account(s)", pool_config.erasure.parity_chunks());
    println!("  Preset: {:?}", pool_config.erasure.preset);
    println!();

    println!("Accounts ({}):", pool_config.accounts.len());
    for account in &pool_config.accounts {
        let status = if account.enabled { "enabled" } else { "disabled" };
        println!("  [{}] {} - {:?} ({})",
            account.account_id,
            account.phone.as_deref().unwrap_or("no phone"),
            account.session_file,
            status
        );
    }
    println!();

    // Try to connect and get live status
    let runtime = tokio::runtime::Runtime::new().map_err(|e| Error::Internal(e.to_string()))?;
    runtime.block_on(async {
        match AccountPool::new(pool_config) {
            Ok(pool) => {
                if let Err(e) = pool.connect_all().await {
                    warn!("Could not connect to all accounts: {}", e);
                }

                let health = pool.health();
                println!("Array Status: {:?}", health.status);
                println!("Healthy accounts: {}/{}", pool.healthy_count(), pool.account_count());

                match health.status {
                    ArrayStatus::Healthy => println!("  All accounts operational, full redundancy."),
                    ArrayStatus::Degraded => println!("  WARNING: Operating with reduced redundancy!"),
                    ArrayStatus::Failed => println!("  CRITICAL: Not enough accounts available!"),
                    ArrayStatus::Rebuilding => println!("  Rebuild in progress..."),
                }

                println!();
                println!("Account Health:");
                for account_health in &health.accounts {
                    println!("  [{}] {:?} - {} ops, {:.1}% error rate",
                        account_health.account_id,
                        account_health.status,
                        account_health.total_operations,
                        account_health.error_rate() * 100.0
                    );
                    if let Some(err) = &account_health.last_error {
                        println!("      Last error: {}", err);
                    }
                }

                pool.disconnect_all().await;
            }
            Err(e) => {
                warn!("Could not create account pool: {}", e);
                println!("Status: Unable to connect ({})", e);
            }
        }
        Ok::<_, Error>(())
    })?;

    Ok(())
}

fn cmd_raid_rebuild(config_path: &PathBuf, account_id: u8) -> Result<()> {
    use tgcryptfs::config::ConfigV2;

    info!("Starting rebuild for account {}...", account_id);

    let config = ConfigV2::load(config_path)?;

    let pool_config = config.pool.ok_or_else(|| {
        Error::InvalidConfig("No pool configuration found.".to_string())
    })?;

    if account_id as usize >= pool_config.accounts.len() {
        return Err(Error::InvalidConfig(format!(
            "Account {} not found. Valid range: 0-{}",
            account_id,
            pool_config.accounts.len() - 1
        )));
    }

    println!("Rebuild for account {} not yet fully implemented.", account_id);
    println!("This will:");
    println!("  1. Mark account {} as rebuilding", account_id);
    println!("  2. For each stripe with a block on this account:");
    println!("     - Download K blocks from other accounts");
    println!("     - Reconstruct the missing block using Reed-Solomon");
    println!("     - Re-upload to account {}", account_id);
    println!("  3. Mark account as healthy when complete");

    Ok(())
}

fn cmd_raid_scrub(config_path: &PathBuf, repair: bool) -> Result<()> {
    use tgcryptfs::config::ConfigV2;

    info!("Starting scrub operation...");
    if repair {
        info!("Repair mode enabled - will fix issues found");
    }

    let config = ConfigV2::load(config_path)?;

    let _pool_config = config.pool.ok_or_else(|| {
        Error::InvalidConfig("No pool configuration found.".to_string())
    })?;

    println!("Scrub operation not yet fully implemented.");
    println!("This will:");
    println!("  1. Iterate through all stored stripes");
    println!("  2. Download all blocks for each stripe");
    println!("  3. Verify Reed-Solomon decoding succeeds");
    println!("  4. Report any inconsistencies");
    if repair {
        println!("  5. Re-upload any missing/corrupted blocks");
    }

    Ok(())
}

fn cmd_raid_add_account(
    config_path: &PathBuf,
    api_id: i32,
    api_hash: String,
    session_file: PathBuf,
    phone: Option<String>,
) -> Result<()> {
    use tgcryptfs::config::ConfigV2;
    use tgcryptfs::raid::config::AccountConfig;

    info!("Adding new account to pool...");

    let mut config = if config_path.exists() {
        ConfigV2::load(config_path)?
    } else {
        ConfigV2::from_env()?
    };

    // Get or create pool config
    let mut pool_config = config.pool.take().unwrap_or_default();

    // Determine next account ID
    let next_id = pool_config.accounts.iter()
        .map(|a| a.account_id)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);

    // Create account config
    let account = AccountConfig::new(next_id, api_id, api_hash, session_file.clone());
    let account = if let Some(p) = phone {
        account.with_phone(p)
    } else {
        account
    };

    pool_config.accounts.push(account);

    // Update erasure config based on account count
    let account_count = pool_config.accounts.len();
    if account_count >= 2 {
        // Default to RAID5-style (can lose 1 account)
        pool_config.erasure.total_chunks = account_count;
        pool_config.erasure.data_chunks = account_count - 1;
        pool_config.erasure.enabled = true;
    }

    config.pool = Some(pool_config);

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    config.save(config_path)?;

    println!("Account added successfully:");
    println!("  Account ID: {}", next_id);
    println!("  Session file: {:?}", session_file);
    println!("  Total accounts: {}", account_count);
    println!();
    println!("Current erasure config:");
    println!("  Data chunks (K): {}", config.pool.as_ref().unwrap().erasure.data_chunks);
    println!("  Total chunks (N): {}", config.pool.as_ref().unwrap().erasure.total_chunks);
    println!();
    println!("Next steps:");
    println!("  1. Run 'tgcryptfs auth --phone <phone>' to authenticate this account");
    println!("  2. Run 'tgcryptfs raid status' to verify the pool");

    Ok(())
}

fn cmd_raid_migrate(config_path: &PathBuf, dry_run: bool, delete_old: bool) -> Result<()> {
    use tgcryptfs::config::ConfigV2;

    info!("Starting migration to erasure-coded storage...");
    if dry_run {
        info!("DRY RUN MODE - no changes will be made");
    }
    if delete_old {
        info!("Will delete old single-account messages after migration");
    }

    let config = ConfigV2::load(config_path)?;

    let pool_config = config.pool.ok_or_else(|| {
        Error::InvalidConfig("No pool configuration found. Add accounts first.".to_string())
    })?;

    if pool_config.accounts.len() < 2 {
        return Err(Error::InvalidConfig(
            "At least 2 accounts required for erasure coding.".to_string()
        ));
    }

    println!("Migration to erasure coding not yet fully implemented.");
    println!();
    println!("This will:");
    println!("  1. Read existing chunk manifests from metadata");
    println!("  2. For each chunk stored on a single account:");
    println!("     - Download the chunk");
    println!("     - Encode into {} blocks using Reed-Solomon", pool_config.erasure.total_chunks);
    println!("     - Upload blocks to {} accounts in parallel", pool_config.accounts.len());
    println!("     - Update manifest with ErasureChunkRef");
    if delete_old {
        println!("  3. Delete old single-account messages");
    }
    println!();
    println!("Accounts configured: {}", pool_config.accounts.len());
    println!("Erasure config: {}-of-{}", pool_config.erasure.data_chunks, pool_config.erasure.total_chunks);

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

fn run_timemachine_command(command: TimemachineCommands, config_path: &PathBuf) -> Result<()> {
    match command {
        TimemachineCommands::Init { size, name } => cmd_timemachine_init(config_path, &size, &name),
        TimemachineCommands::Mount { mount_point } => cmd_timemachine_mount(config_path, mount_point),
        TimemachineCommands::Unmount => cmd_timemachine_unmount(config_path),
        TimemachineCommands::Status => cmd_timemachine_status(config_path),
        TimemachineCommands::Sync { full } => cmd_timemachine_sync(config_path, full),
    }
}

fn cmd_timemachine_init(config_path: &PathBuf, size: &str, name: &str) -> Result<()> {
    let config = Config::load(config_path)?;
    let tm_dir = config.data_dir.join("timemachine");
    let sparsebundle_path = tm_dir.join(format!("{}.sparsebundle", name));

    info!("Initializing Time Machine backup volume...");
    info!("  Name: {}", name);
    info!("  Size: {}", size);
    info!("  Location: {:?}", sparsebundle_path);

    // Create Time Machine directory
    std::fs::create_dir_all(&tm_dir)?;

    // Create sparsebundle using hdiutil (macOS only)
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("hdiutil")
            .args([
                "create",
                "-size", size,
                "-type", "SPARSEBUNDLE",
                "-fs", "APFS",
                "-volname", name,
                sparsebundle_path.to_str().unwrap(),
            ])
            .output()?;

        if !output.status.success() {
            return Err(Error::Internal(format!(
                "Failed to create sparsebundle: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        println!("Time Machine volume created successfully!");
        println!();
        println!("Next steps:");
        println!("  1. Run 'tgcryptfs timemachine mount' to mount the volume");
        println!("  2. Run 'sudo tmutil setdestination /Volumes/{}' to set as Time Machine destination", name);
        println!("  3. Time Machine will start backing up automatically");
    }

    #[cfg(not(target_os = "macos"))]
    {
        println!("Time Machine is only supported on macOS.");
        println!("On Linux, use 'rsync' for backups instead.");
    }

    Ok(())
}

fn cmd_timemachine_mount(config_path: &PathBuf, mount_point: Option<PathBuf>) -> Result<()> {
    let config = Config::load(config_path)?;
    let tm_dir = config.data_dir.join("timemachine");

    // Find the first sparsebundle
    let sparsebundle = std::fs::read_dir(&tm_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().map(|ext| ext == "sparsebundle").unwrap_or(false))
        .ok_or_else(|| Error::Internal("No Time Machine volume found. Run 'tgcryptfs timemachine init' first.".to_string()))?;

    info!("Mounting Time Machine volume: {:?}", sparsebundle.path());

    #[cfg(target_os = "macos")]
    {
        let mut args = vec!["attach".to_string(), sparsebundle.path().to_string_lossy().to_string()];

        if let Some(mp) = mount_point {
            args.push("-mountpoint".to_string());
            args.push(mp.to_string_lossy().to_string());
        }

        let output = std::process::Command::new("hdiutil")
            .args(&args)
            .output()?;

        if !output.status.success() {
            return Err(Error::Internal(format!(
                "Failed to mount sparsebundle: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        println!("Time Machine volume mounted successfully!");
        println!("{}", String::from_utf8_lossy(&output.stdout));
    }

    #[cfg(not(target_os = "macos"))]
    {
        println!("Time Machine mounting is only supported on macOS.");
    }

    Ok(())
}

fn cmd_timemachine_unmount(_config_path: &PathBuf) -> Result<()> {
    info!("Unmounting Time Machine volume...");

    #[cfg(target_os = "macos")]
    {
        // Find mounted Time Machine volumes
        let _output = std::process::Command::new("hdiutil")
            .args(["info", "-plist"])
            .output()?;

        // For simplicity, try to unmount /Volumes/TGCryptFS-TM
        let output = std::process::Command::new("hdiutil")
            .args(["detach", "/Volumes/TGCryptFS-TM"])
            .output()?;

        if !output.status.success() {
            // Try alternate name
            let _ = std::process::Command::new("hdiutil")
                .args(["detach", "/Volumes/TM-Backup"])
                .output();
        }

        println!("Time Machine volume unmounted.");
    }

    #[cfg(not(target_os = "macos"))]
    {
        println!("Time Machine unmounting is only supported on macOS.");
    }

    Ok(())
}

fn cmd_timemachine_status(config_path: &PathBuf) -> Result<()> {
    let config = Config::load(config_path)?;
    let tm_dir = config.data_dir.join("timemachine");

    println!("Time Machine Status");
    println!("===================");
    println!();

    // List sparsebundles
    println!("Backup Volumes:");
    if tm_dir.exists() {
        for entry in std::fs::read_dir(&tm_dir)? {
            let entry = entry?;
            if entry.path().extension().map(|ext| ext == "sparsebundle").unwrap_or(false) {
                let meta = entry.metadata()?;
                println!("  {:?}", entry.path().file_name().unwrap());
                println!("    Size: {} MB", meta.len() / 1024 / 1024);
            }
        }
    } else {
        println!("  No backup volumes found.");
        println!("  Run 'tgcryptfs timemachine init' to create one.");
    }

    #[cfg(target_os = "macos")]
    {
        println!();
        println!("Time Machine Destinations:");
        let output = std::process::Command::new("tmutil")
            .arg("destinationinfo")
            .output();

        match output {
            Ok(out) => println!("{}", String::from_utf8_lossy(&out.stdout)),
            Err(_) => println!("  Unable to query Time Machine destinations."),
        }
    }

    Ok(())
}

fn cmd_timemachine_sync(config_path: &PathBuf, full: bool) -> Result<()> {
    let config = Config::load(config_path)?;
    let tm_dir = config.data_dir.join("timemachine");

    info!("Syncing Time Machine volume to cloud...");

    if full {
        info!("Performing full sync...");
    }

    // Find sparsebundle
    let sparsebundle = std::fs::read_dir(&tm_dir)?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().map(|ext| ext == "sparsebundle").unwrap_or(false));

    match sparsebundle {
        Some(sb) => {
            println!("Syncing: {:?}", sb.path());
            println!("Note: Sync to tgcryptfs cloud storage not yet implemented.");
            println!("      The sparsebundle is stored locally at: {:?}", sb.path());
            println!();
            println!("To manually sync to cloud storage:");
            println!("  rsync -avh {:?}/ ~/mnt/tgcryptfs/timemachine/", sb.path());
        }
        None => {
            println!("No Time Machine volume found. Run 'tgcryptfs timemachine init' first.");
        }
    }

    Ok(())
}
