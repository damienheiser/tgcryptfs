//! Overlay filesystem configuration

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for overlay filesystem
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayConfig {
    /// Path to the lower (read-only) layer
    pub lower_path: PathBuf,

    /// Path to the upper (writable) layer
    pub upper_path: PathBuf,

    /// Path to whiteout database
    pub whiteout_db_path: PathBuf,

    /// Behavior when file exists in both layers
    pub conflict_behavior: ConflictBehavior,

    /// Whether to follow symlinks in lower layer
    pub follow_symlinks: bool,

    /// File patterns to exclude from lower layer
    pub exclude_patterns: Vec<String>,

    /// Whether to make opaque directories on first write
    pub auto_opaque_dirs: bool,

    /// Copy-up threshold: files smaller than this are copied entirely
    /// on first modification (0 = always copy-up)
    pub copy_up_threshold: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ConflictBehavior {
    /// Upper layer always wins (default overlay behavior)
    #[default]
    UpperWins,
    /// Error if file exists in both (stricter mode)
    Error,
    /// Merge directories, upper files win
    MergeDirectories,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| home.join(".local/share"))
            .join("tgcryptfs");

        OverlayConfig {
            lower_path: home,
            upper_path: data_dir.join("overlay_upper"),
            whiteout_db_path: data_dir.join("overlay_whiteout.db"),
            conflict_behavior: ConflictBehavior::UpperWins,
            follow_symlinks: true,
            exclude_patterns: vec![
                ".git".to_string(),
                ".cache".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                ".DS_Store".to_string(),
                ".Trash".to_string(),
                "Library/Caches".to_string(),
            ],
            auto_opaque_dirs: true,
            copy_up_threshold: 10 * 1024 * 1024, // 10MB
        }
    }
}

impl OverlayConfig {
    /// Create config with specified lower path
    pub fn with_lower_path(lower_path: PathBuf) -> Self {
        Self {
            lower_path,
            ..Default::default()
        }
    }

    /// Create config with specified lower and upper paths
    pub fn with_paths(lower_path: PathBuf, upper_path: PathBuf) -> Self {
        Self {
            lower_path,
            upper_path,
            ..Default::default()
        }
    }

    /// Check if a path matches any exclude pattern
    pub fn is_excluded(&self, path: &std::path::Path) -> bool {
        let path_str = path.to_string_lossy();
        for pattern in &self.exclude_patterns {
            if path_str.contains(pattern) {
                return true;
            }
        }
        false
    }
}
