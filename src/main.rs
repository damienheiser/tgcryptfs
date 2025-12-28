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

        Commands::Migrate {
            password_file,
            dry_run,
            force,
        } => cmd_migrate(config_path, password_file, dry_run, force),
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

/// Expand ~ to home directory
fn expand_tilde(path: &PathBuf) -> PathBuf {
    if path.starts_with("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(path.strip_prefix("~").unwrap());
        }
    }
    path.clone()
}
