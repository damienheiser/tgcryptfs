//! Lower layer pass-through interface
//!
//! Provides read-only access to the local filesystem (lower layer).

use crate::error::{Error, Result};
use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::OverlayConfig;

/// Directory entry from lower layer
#[derive(Debug, Clone)]
pub struct LowerDirEntry {
    pub name: OsString,
    pub file_type: fs::FileType,
    pub ino: u64,
}

/// Pass-through interface to lower (local) filesystem
pub struct LowerLayer {
    /// Root path of lower layer
    root: PathBuf,
    /// Configuration
    config: OverlayConfig,
}

impl LowerLayer {
    /// Create a new lower layer interface
    pub fn new(root: PathBuf, config: OverlayConfig) -> Result<Self> {
        if !root.exists() {
            return Err(Error::PathNotFound(root.to_string_lossy().to_string()));
        }
        Ok(Self { root, config })
    }

    /// Get the root path
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a virtual path to lower layer absolute path
    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            // Strip leading / for joining
            let relative = path.strip_prefix("/").unwrap_or(path);
            self.root.join(relative)
        } else {
            self.root.join(path)
        }
    }

    /// Convert absolute lower path back to virtual path
    pub fn to_virtual(&self, lower_path: &Path) -> Option<PathBuf> {
        lower_path
            .strip_prefix(&self.root)
            .ok()
            .map(|p| PathBuf::from("/").join(p))
    }

    /// Check if path exists in lower layer
    pub fn exists(&self, path: &Path) -> bool {
        let resolved = self.resolve(path);
        resolved.exists() && !self.is_excluded(path)
    }

    /// Check if path is excluded by config
    pub fn is_excluded(&self, path: &Path) -> bool {
        self.config.is_excluded(path)
    }

    /// Get metadata for a path
    pub fn metadata(&self, path: &Path) -> Result<Metadata> {
        let resolved = self.resolve(path);
        if self.config.follow_symlinks {
            fs::metadata(&resolved)
        } else {
            fs::symlink_metadata(&resolved)
        }
        .map_err(|e| Error::Io(e))
    }

    /// Check if path is a directory
    pub fn is_dir(&self, path: &Path) -> bool {
        self.metadata(path).map(|m| m.is_dir()).unwrap_or(false)
    }

    /// Check if path is a file
    pub fn is_file(&self, path: &Path) -> bool {
        self.metadata(path).map(|m| m.is_file()).unwrap_or(false)
    }

    /// Check if path is a symlink
    pub fn is_symlink(&self, path: &Path) -> bool {
        let resolved = self.resolve(path);
        fs::symlink_metadata(&resolved)
            .map(|m| m.is_symlink())
            .unwrap_or(false)
    }

    /// Read file content at offset
    pub fn read(&self, path: &Path, offset: u64, size: u32) -> Result<Vec<u8>> {
        let resolved = self.resolve(path);
        let mut file = fs::File::open(&resolved).map_err(|e| Error::Io(e))?;

        file.seek(SeekFrom::Start(offset))
            .map_err(|e| Error::Io(e))?;

        let mut buffer = vec![0u8; size as usize];
        let bytes_read = file.read(&mut buffer).map_err(|e| Error::Io(e))?;
        buffer.truncate(bytes_read);

        Ok(buffer)
    }

    /// Read entire file
    pub fn read_all(&self, path: &Path) -> Result<Vec<u8>> {
        let resolved = self.resolve(path);
        fs::read(&resolved).map_err(|e| Error::Io(e))
    }

    /// Read directory entries
    pub fn readdir(&self, path: &Path) -> Result<Vec<LowerDirEntry>> {
        let resolved = self.resolve(path);
        let mut entries = Vec::new();

        for entry in fs::read_dir(&resolved).map_err(|e| Error::Io(e))? {
            let entry = entry.map_err(|e| Error::Io(e))?;
            let name = entry.file_name();

            // Skip excluded entries
            let entry_path = path.join(&name);
            if self.is_excluded(&entry_path) {
                continue;
            }

            let file_type = entry.file_type().map_err(|e| Error::Io(e))?;
            let metadata = entry.metadata().map_err(|e| Error::Io(e))?;

            #[cfg(unix)]
            let ino = {
                use std::os::unix::fs::MetadataExt;
                metadata.ino()
            };
            #[cfg(not(unix))]
            let ino = 0;

            entries.push(LowerDirEntry {
                name,
                file_type,
                ino,
            });
        }

        Ok(entries)
    }

    /// Read symlink target
    pub fn readlink(&self, path: &Path) -> Result<PathBuf> {
        let resolved = self.resolve(path);
        fs::read_link(&resolved).map_err(|e| Error::Io(e))
    }

    /// Get file size
    pub fn size(&self, path: &Path) -> Result<u64> {
        self.metadata(path).map(|m| m.len())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_lower_layer() {
        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.txt");
        fs::write(&test_file, b"hello world").unwrap();

        let config = OverlayConfig::with_lower_path(dir.path().to_path_buf());
        let lower = LowerLayer::new(dir.path().to_path_buf(), config).unwrap();

        assert!(lower.exists(Path::new("test.txt")));
        assert!(!lower.exists(Path::new("nonexistent.txt")));

        let content = lower.read(Path::new("test.txt"), 0, 100).unwrap();
        assert_eq!(content, b"hello world");
    }

    #[test]
    fn test_readdir() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("file1.txt"), b"1").unwrap();
        fs::write(dir.path().join("file2.txt"), b"2").unwrap();
        fs::create_dir(dir.path().join("subdir")).unwrap();

        let config = OverlayConfig::with_lower_path(dir.path().to_path_buf());
        let lower = LowerLayer::new(dir.path().to_path_buf(), config).unwrap();

        let entries = lower.readdir(Path::new("")).unwrap();
        assert_eq!(entries.len(), 3);
    }
}
