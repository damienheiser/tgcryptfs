# tgcryptfs Deployment Guide

This document describes how to deploy and configure tgcryptfs on a new machine.

## Prerequisites

- **Rust 1.70+** (for building from source)
- **FUSE libraries**:
  - Linux: `libfuse3-dev` (Debian/Ubuntu) or `fuse3-devel` (RHEL/Fedora)
  - macOS: [macFUSE](https://osxfuse.github.io/)
- **Telegram API credentials** from [my.telegram.org](https://my.telegram.org)

## Installation

### Option 1: Pre-built Binaries

Download from the [releases page](https://github.com/damienheiser/tgcryptfs/releases):

```bash
# macOS (Apple Silicon)
curl -L https://github.com/damienheiser/tgcryptfs/releases/latest/download/tgcryptfs-macos-aarch64 -o tgcryptfs

# Linux (x86_64)
curl -L https://github.com/damienheiser/tgcryptfs/releases/latest/download/tgcryptfs-linux-x86_64 -o tgcryptfs

# Linux (ARM64/Asahi)
curl -L https://github.com/damienheiser/tgcryptfs/releases/latest/download/tgcryptfs-linux-aarch64 -o tgcryptfs

chmod +x tgcryptfs
sudo mv tgcryptfs /usr/local/bin/
```

### Option 2: Build from Source

```bash
# Install dependencies (Debian/Ubuntu)
sudo apt-get install -y libfuse3-dev pkg-config libsqlite3-dev

# Install dependencies (macOS)
brew install macfuse

# Clone and build
git clone https://github.com/damienheiser/tgcryptfs.git
cd tgcryptfs
cargo build --release
sudo cp target/release/tgcryptfs /usr/local/bin/
```

## Initial Setup

### 1. Initialize Configuration

```bash
tgcryptfs init --api-id YOUR_API_ID --api-hash YOUR_API_HASH
```

This creates:
- `~/.config/tgcryptfs/config.json` - Main configuration file
- `~/.local/share/tgcryptfs/` - Data directory (metadata, session)
- `~/.cache/tgcryptfs/` - Cache directory

### 2. Generate Encryption Key

Generate a strong random password for encryption:

```bash
# Create secure password file
mkdir -p ~/.local/share/tgcryptfs
openssl rand -base64 32 > ~/.local/share/tgcryptfs/encryption.key
chmod 600 ~/.local/share/tgcryptfs/encryption.key
```

**IMPORTANT**: Back up this key securely. Without it, your data cannot be decrypted.

### 3. Authenticate with Telegram

```bash
tgcryptfs auth --phone +1234567890
```

You'll receive a code via Telegram. If 2FA is enabled, you'll also need your password.

### 4. Create Mount Point

```bash
mkdir -p ~/tgcryptfs
```

### 5. Mount the Filesystem

```bash
# Foreground (for testing)
tgcryptfs mount ~/tgcryptfs --password-file ~/.local/share/tgcryptfs/encryption.key -f

# Background (for normal use)
tgcryptfs mount ~/tgcryptfs --password-file ~/.local/share/tgcryptfs/encryption.key
```

## Server Deployment

For headless servers, use systemd to manage the service.

### 1. Create System Directories

```bash
# Configuration (root-owned for security)
sudo mkdir -p /etc/tgcryptfs
sudo chmod 700 /etc/tgcryptfs

# Data directory
sudo mkdir -p /var/lib/tgcryptfs
sudo chown $USER:$USER /var/lib/tgcryptfs

# Cache directory
sudo mkdir -p /var/cache/tgcryptfs
sudo chown $USER:$USER /var/cache/tgcryptfs

# Mount point
sudo mkdir -p /mnt/tgcryptfs
sudo chown $USER:$USER /mnt/tgcryptfs
```

### 2. Generate Encryption Key

```bash
openssl rand -base64 32 | sudo tee /etc/tgcryptfs/encryption.key > /dev/null
sudo chmod 600 /etc/tgcryptfs/encryption.key
```

### 3. Create Configuration

Create `/home/$USER/.config/tgcryptfs/config.json`:

```json
{
  "telegram": {
    "api_id": YOUR_API_ID,
    "api_hash": "YOUR_API_HASH",
    "phone": "+1234567890",
    "session_file": "/var/lib/tgcryptfs/hostname.session",
    "max_concurrent_uploads": 4,
    "max_concurrent_downloads": 8,
    "retry_attempts": 5,
    "retry_base_delay_ms": 1000
  },
  "encryption": {
    "argon2_memory_kib": 65536,
    "argon2_iterations": 3,
    "argon2_parallelism": 4,
    "salt": ""
  },
  "cache": {
    "max_size": 10737418240,
    "cache_dir": "/var/cache/tgcryptfs",
    "prefetch_enabled": true,
    "prefetch_count": 3,
    "eviction_policy": "Lru"
  },
  "chunk": {
    "chunk_size": 52428800,
    "compression_enabled": true,
    "compression_threshold": 1024,
    "dedup_enabled": true
  },
  "mount": {
    "mount_point": "/mnt/tgcryptfs",
    "allow_other": false,
    "allow_root": true,
    "default_file_mode": 420,
    "default_dir_mode": 493,
    "uid": 1000,
    "gid": 1000
  },
  "versioning": {
    "enabled": true,
    "max_versions": 10,
    "auto_snapshot": false,
    "snapshot_interval_secs": 3600,
    "max_snapshots": 50
  },
  "data_dir": "/var/lib/tgcryptfs"
}
```

**Note**: The `salt` field is auto-populated on first mount.

### 4. Authenticate

```bash
tgcryptfs auth --phone +1234567890
```

### 5. Create Systemd Service

Create `/etc/systemd/system/tgcryptfs.service`:

```ini
[Unit]
Description=tgcryptfs Encrypted Cloud Filesystem
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=hedon
Group=hedon
ExecStart=/usr/local/bin/tgcryptfs mount /mnt/tgcryptfs -f --password-file /etc/tgcryptfs/encryption.key
ExecStop=/usr/bin/fusermount -u /mnt/tgcryptfs
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

### 6. Enable and Start

```bash
sudo systemctl daemon-reload
sudo systemctl enable tgcryptfs
sudo systemctl start tgcryptfs

# Check status
sudo systemctl status tgcryptfs
```

## Configuration Reference

### Telegram Settings

| Field | Description | Default |
|-------|-------------|---------|
| `api_id` | Telegram API ID | Required |
| `api_hash` | Telegram API hash | Required |
| `phone` | Phone number for auth | Optional |
| `session_file` | Path to session file | `~/.local/share/tgcryptfs/<hostname>.session` |
| `max_concurrent_uploads` | Parallel upload limit | 3 |
| `max_concurrent_downloads` | Parallel download limit | 5 |
| `retry_attempts` | Upload/download retries | 3 |
| `retry_base_delay_ms` | Base delay between retries | 1000 |

### Encryption Settings

| Field | Description | Default |
|-------|-------------|---------|
| `argon2_memory_kib` | Argon2 memory cost (KiB) | 65536 (64MB) |
| `argon2_iterations` | Argon2 time cost | 3 |
| `argon2_parallelism` | Argon2 parallelism | 4 |
| `salt` | KDF salt (hex, auto-generated) | Generated on first use |

### Cache Settings

| Field | Description | Default |
|-------|-------------|---------|
| `max_size` | Maximum cache size (bytes) | 10737418240 (10GB) |
| `cache_dir` | Cache directory path | `~/.cache/tgcryptfs` |
| `prefetch_enabled` | Enable read-ahead | true |
| `prefetch_count` | Number of chunks to prefetch | 3 |
| `eviction_policy` | Cache eviction policy | "Lru" |

### Chunk Settings

| Field | Description | Default |
|-------|-------------|---------|
| `chunk_size` | Size of each chunk (bytes) | 52428800 (50MB) |
| `compression_enabled` | Enable LZ4 compression | true |
| `compression_threshold` | Min size to compress (bytes) | 1024 |
| `dedup_enabled` | Enable deduplication | true |

### Mount Settings

| Field | Description | Default |
|-------|-------------|---------|
| `mount_point` | Default mount location | `~/tgcryptfs` |
| `allow_other` | Allow other users to access | false |
| `allow_root` | Allow root to access | false |
| `default_file_mode` | File permissions (octal) | 420 (0644) |
| `default_dir_mode` | Directory permissions (octal) | 493 (0755) |
| `uid` | Owner UID | Current user |
| `gid` | Owner GID | Current group |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `TELEGRAM_APP_ID` | Telegram API ID (overrides config) |
| `TELEGRAM_APP_HASH` | Telegram API hash (overrides config) |
| `TELEGRAM_PHONE` | Phone number for auth |
| `TGCRYPTFS_CACHE_SIZE` | Cache size in bytes |
| `TGCRYPTFS_CHUNK_SIZE` | Chunk size in bytes |
| `TGCRYPTFS_MACHINE_NAME` | Machine identifier for distributed mode |

## Troubleshooting

### "Decryption failed - data corrupted or wrong key"

This error occurs when:
1. **Wrong password** - Verify the password file contents
2. **HKDF mismatch** - Data was encrypted with different HKDF strings (see Migration section)
3. **Corrupted metadata** - The local metadata database may be corrupted

### "Resource temporarily unavailable"

The metadata database is locked by another process. Check:
```bash
lsof /var/lib/tgcryptfs/metadata.db
```

### FUSE errors on macOS

1. Allow the kernel extension in System Preferences > Security & Privacy
2. Reboot after installing macFUSE
3. Check that macFUSE is loaded: `kextstat | grep fuse`

### Connection issues

1. Verify Telegram credentials
2. Check firewall rules for Telegram's servers
3. Try re-authenticating: `tgcryptfs auth --phone +1234567890`

## Security Recommendations

1. **Protect the encryption key**: Use `chmod 600` and store securely
2. **Protect the session file**: Contains Telegram authentication
3. **Use full-disk encryption**: The cache stores decrypted data
4. **Regular backups**: Keep offline copies of critical data
5. **Strong password**: If not using a generated key file

## File Locations Summary

| Purpose | Linux (Server) | macOS (Desktop) |
|---------|----------------|-----------------|
| Config | `~/.config/tgcryptfs/config.json` | `~/.config/tgcryptfs/config.json` |
| Session | `/var/lib/tgcryptfs/*.session` | `~/.local/share/tgcryptfs/*.session` |
| Metadata | `/var/lib/tgcryptfs/metadata.db/` | `~/.local/share/tgcryptfs/metadata.db/` |
| Cache | `/var/cache/tgcryptfs/` | `~/.cache/tgcryptfs/` |
| Password | `/etc/tgcryptfs/encryption.key` | `~/.local/share/tgcryptfs/encryption.key` |
| Mount | `/mnt/tgcryptfs` | `~/tgcryptfs` |
