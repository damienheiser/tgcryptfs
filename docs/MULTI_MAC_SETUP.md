# Setting Up tgcryptfs on Multiple Macs

This guide walks through setting up a second Mac to access the same tgcryptfs filesystem that is already running on your primary Mac.

## Overview

**Scenario**: Mac 1 ("pleasure") is running tgcryptfs syncing your home directory to Telegram cloud. You want Mac 2 to access the same encrypted filesystem.

**Important**: As of v3.1.0, tgcryptfs does not have real-time sync between machines. This guide covers the safest current approach.

---

## 1. Prerequisites on Mac 2

### 1.1 Install macFUSE

macFUSE is required for FUSE filesystem support on macOS.

```bash
# Option A: Download from official site
open https://osxfuse.github.io/

# Option B: Install via Homebrew
brew install --cask macfuse
```

After installation:
1. Go to **System Settings > Privacy & Security**
2. Allow the macFUSE kernel extension
3. **Reboot your Mac** (required)

Verify installation:
```bash
kextstat | grep -i fuse
# Should show: com.github.osxfuse.filesystems.macfuse
```

### 1.2 Install tgcryptfs

```bash
# Option A: Pre-built binary (Apple Silicon)
curl -L https://github.com/damienheiser/tgcryptfs/releases/latest/download/tgcryptfs-macos-aarch64 -o tgcryptfs
chmod +x tgcryptfs
sudo mv tgcryptfs /usr/local/bin/

# Option B: Build from source
brew install macfuse
git clone https://github.com/damienheiser/tgcryptfs.git
cd tgcryptfs
cargo build --release
sudo cp target/release/tgcryptfs /usr/local/bin/
```

Verify:
```bash
tgcryptfs --version
```

---

## 2. Configuration Transfer from Mac 1

You need to copy three categories of files from Mac 1 to Mac 2.

### 2.1 Files to Copy

| File | Mac 1 Location | Purpose |
|------|----------------|---------|
| Encryption key | `~/.local/share/tgcryptfs/encryption.key` | Decrypts all data |
| Config file | `~/.config/tgcryptfs/config.json` | API credentials, settings |
| Session file | `~/.local/share/tgcryptfs/*.session` | Telegram authentication |

### 2.2 Create Directories on Mac 2

```bash
mkdir -p ~/.config/tgcryptfs
mkdir -p ~/.local/share/tgcryptfs
mkdir -p ~/.cache/tgcryptfs
```

### 2.3 Secure Transfer Methods

**Option A: AirDrop (recommended for local transfer)**
```bash
# On Mac 1, right-click files and AirDrop to Mac 2
```

**Option B: SCP over secure network**
```bash
# On Mac 2, pull files from Mac 1
scp mac1:~/.local/share/tgcryptfs/encryption.key ~/.local/share/tgcryptfs/
scp mac1:~/.config/tgcryptfs/config.json ~/.config/tgcryptfs/
scp mac1:~/.local/share/tgcryptfs/*.session ~/.local/share/tgcryptfs/
```

**Option C: Encrypted USB drive**
```bash
# On Mac 1: Copy to encrypted volume
# On Mac 2: Copy from encrypted volume to correct locations
```

### 2.4 Set Correct Permissions

```bash
chmod 600 ~/.local/share/tgcryptfs/encryption.key
chmod 600 ~/.local/share/tgcryptfs/*.session
chmod 600 ~/.config/tgcryptfs/config.json
```

---

## 3. Telegram Session Management

### 3.1 Can the Same Session Be Used on Both Macs?

**Yes, but with caveats:**

- Telegram allows multiple sessions per account
- The same session file CAN be used on multiple devices
- However, concurrent access may cause issues
- Telegram may invalidate sessions if it detects suspicious activity

### 3.2 Recommended: Create New Session on Mac 2

For stability, authenticate Mac 2 separately:

```bash
# On Mac 2
# First, update config to use a unique session file
# Edit ~/.config/tgcryptfs/config.json:
# Change "session_file" to "~/.local/share/tgcryptfs/mac2.session"

# Then authenticate
tgcryptfs auth --phone +1234567890
```

You will receive a code via Telegram. Enter it when prompted.

### 3.3 Session File Naming Convention

Use machine-specific session files to avoid conflicts:

```json
{
  "telegram": {
    "session_file": "~/.local/share/tgcryptfs/pleasure.session"
  }
}
```

On Mac 2:
```json
{
  "telegram": {
    "session_file": "~/.local/share/tgcryptfs/mac2.session"
  }
}
```

---

## 4. Mounting on Mac 2

### 4.1 Create Mount Point

```bash
mkdir -p ~/tgcryptfs
```

### 4.2 Basic Mount Command

```bash
# Foreground (for testing)
tgcryptfs mount ~/tgcryptfs \
  --password-file ~/.local/share/tgcryptfs/encryption.key \
  -f

# Background (for normal use)
tgcryptfs mount ~/tgcryptfs \
  --password-file ~/.local/share/tgcryptfs/encryption.key
```

### 4.3 Setting Up launchd for Auto-Mount

Create `~/Library/LaunchAgents/com.tgcryptfs.mount.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.tgcryptfs.mount</string>

    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/tgcryptfs</string>
        <string>mount</string>
        <string>/Users/YOUR_USERNAME/tgcryptfs</string>
        <string>--password-file</string>
        <string>/Users/YOUR_USERNAME/.local/share/tgcryptfs/encryption.key</string>
        <string>-f</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>NetworkState</key>
        <true/>
    </dict>

    <key>StandardOutPath</key>
    <string>/tmp/tgcryptfs.log</string>

    <key>StandardErrorPath</key>
    <string>/tmp/tgcryptfs.err</string>
</dict>
</plist>
```

Load the service:
```bash
launchctl load ~/Library/LaunchAgents/com.tgcryptfs.mount.plist
```

### 4.4 Unmounting

```bash
tgcryptfs unmount ~/tgcryptfs
# or
umount ~/tgcryptfs
```

---

## 5. Sync Strategy

### 5.1 Option A: Read-Only Mode on Mac 2 (SAFEST)

This is the recommended approach for avoiding conflicts.

**Mac 1 (pleasure)**: Full read/write - primary machine
**Mac 2**: Read-only access

To enforce read-only on Mac 2, mount with FUSE read-only option:
```bash
# Currently requires modifying mount options in code
# Workaround: Only read files, don't write
```

**Benefits**:
- No conflict potential
- Mac 1 remains authoritative source
- Mac 2 can read all synced data

### 5.2 Option B: Full Read-Write (Conflict Risk)

Both Macs can read and write, but:

**Risks**:
- Concurrent writes to same file = data loss
- No automatic conflict resolution yet
- Metadata can become inconsistent

**Mitigations**:
- Never edit same file on both Macs simultaneously
- Use file locking conventions (e.g., `.lock` files)
- Run `tgcryptfs sync` before switching machines

### 5.3 Option C: Using Unison for Bidirectional Sync

For true bidirectional sync with conflict detection:

```bash
# Install Unison
brew install unison

# Create Unison profile ~/.unison/tgcryptfs.prf
# root = /Users/user/tgcryptfs
# root = ssh://mac1//Users/user/tgcryptfs
# prefer = newer
# batch = true

# Run sync
unison tgcryptfs
```

### 5.4 Future: Master-Replica Mode

tgcryptfs v2 config supports master-replica mode:

```yaml
# Mac 1 config (master)
distribution:
  mode: master-replica
  cluster_id: "home-sync"
  master_replica:
    role: master
    master_id: "pleasure"
    sync_interval_secs: 60

# Mac 2 config (replica)
distribution:
  mode: master-replica
  cluster_id: "home-sync"
  master_replica:
    role: replica
    master_id: "pleasure"
    sync_interval_secs: 60
```

**Note**: This is defined in config but sync logic is not yet fully implemented.

---

## 6. Current Limitations

| Limitation | Impact | Workaround |
|------------|--------|------------|
| No real-time sync | Changes don't appear instantly on other Mac | Manual sync, wait for cloud propagation |
| No conflict resolution | Concurrent edits may lose data | Use read-only on secondary, coordinate usage |
| Metadata sync lag | File listings may be stale | Remount to refresh metadata |
| Session sharing risks | Telegram may flag suspicious activity | Use separate session files |

### 6.1 Manual Sync Workflow

Until real-time sync is implemented:

1. **Before working on Mac 2**: Ensure Mac 1 has synced recent changes
2. **On Mac 2**: Unmount and remount to refresh metadata
3. **Before switching back to Mac 1**: Stop using Mac 2, allow sync to complete
4. **On Mac 1**: Unmount and remount to see Mac 2's changes

---

## 7. Complete Step-by-Step Commands

### On Mac 1 (Source Machine)

```bash
# 1. Verify current setup
tgcryptfs status

# 2. Note the file locations
ls -la ~/.config/tgcryptfs/
ls -la ~/.local/share/tgcryptfs/

# 3. Export files for transfer (adjust paths as needed)
tar -czvf ~/tgcryptfs-config-export.tar.gz \
  ~/.config/tgcryptfs/config.json \
  ~/.local/share/tgcryptfs/encryption.key \
  ~/.local/share/tgcryptfs/*.session

# 4. Transfer to Mac 2 via secure method
# (AirDrop, scp, encrypted USB)
```

### On Mac 2 (Target Machine)

```bash
# 1. Install macFUSE
brew install --cask macfuse
# REBOOT REQUIRED

# 2. Install tgcryptfs
curl -L https://github.com/damienheiser/tgcryptfs/releases/latest/download/tgcryptfs-macos-aarch64 -o tgcryptfs
chmod +x tgcryptfs
sudo mv tgcryptfs /usr/local/bin/

# 3. Create directories
mkdir -p ~/.config/tgcryptfs
mkdir -p ~/.local/share/tgcryptfs
mkdir -p ~/.cache/tgcryptfs
mkdir -p ~/tgcryptfs

# 4. Extract transferred config
cd ~
tar -xzvf tgcryptfs-config-export.tar.gz

# 5. Set permissions
chmod 600 ~/.local/share/tgcryptfs/encryption.key
chmod 600 ~/.local/share/tgcryptfs/*.session
chmod 600 ~/.config/tgcryptfs/config.json

# 6. Update config for this machine (optional but recommended)
# Edit ~/.config/tgcryptfs/config.json
# - Change session_file to unique name: "mac2.session"
# - Update any paths if different

# 7. Authenticate (if using new session file)
tgcryptfs auth --phone +1234567890
# Enter code when received

# 8. Test connection
tgcryptfs status

# 9. Mount filesystem (foreground for testing)
tgcryptfs mount ~/tgcryptfs \
  --password-file ~/.local/share/tgcryptfs/encryption.key \
  -f

# 10. In another terminal, verify
ls ~/tgcryptfs

# 11. If working, mount in background
# Ctrl+C the foreground mount, then:
tgcryptfs mount ~/tgcryptfs \
  --password-file ~/.local/share/tgcryptfs/encryption.key

# 12. Verify mount
mount | grep tgcryptfs
df -h ~/tgcryptfs
```

### Verification Checklist

```bash
# Check config is valid
tgcryptfs status

# Check Telegram connection
# Should show "connected and authorized"

# Check mount
mount | grep tgcryptfs
# Should show: tgcryptfs on /Users/.../tgcryptfs

# Check files are accessible
ls ~/tgcryptfs

# Check cache is working
tgcryptfs cache
# Should show cache statistics
```

---

## 8. Troubleshooting

### "Decryption failed"
- Wrong encryption key copied
- Key file corrupted during transfer
- **Fix**: Re-copy encryption.key from Mac 1

### "Not authorized"
- Session expired or invalidated
- **Fix**: Run `tgcryptfs auth --phone <your_phone>`

### "mount_macfuse: mount point is itself on a macfuse volume"
- Trying to mount inside existing mount
- **Fix**: Choose different mount point

### "Resource temporarily unavailable"
- Metadata database locked
- **Fix**: Check for other tgcryptfs processes: `ps aux | grep tgcryptfs`

### Files not appearing on Mac 2
- Sync lag from Telegram cloud
- **Fix**: Wait a few minutes, then unmount and remount

### macFUSE kernel extension not loaded
```bash
# Check if loaded
kextstat | grep fuse

# If not loaded, check System Settings > Privacy & Security
# May need to allow the extension and reboot
```

---

## 9. Security Considerations

1. **Encryption key is the crown jewel** - Anyone with this key and your Telegram session can decrypt all data
2. **Never transmit key unencrypted** - Use AirDrop, encrypted USB, or encrypted channels
3. **Use separate session files** - Reduces risk of session invalidation
4. **Consider FileVault** - Enable full-disk encryption on both Macs
5. **Secure your Telegram account** - Enable 2FA on your Telegram account

---

## 10. Environment Variables

These can be set instead of using config files:

```bash
export TELEGRAM_APP_ID="your_api_id"
export TELEGRAM_APP_HASH="your_api_hash"
export TELEGRAM_PHONE="+1234567890"
export TGCRYPTFS_MACHINE_NAME="mac2"
```

Add to `~/.zshrc` or `~/.bashrc` for persistence.

---

## Summary

| Step | Command |
|------|---------|
| Install macFUSE | `brew install --cask macfuse` + reboot |
| Install tgcryptfs | Download binary or `cargo build --release` |
| Copy config | Transfer config.json, encryption.key, session |
| Set permissions | `chmod 600` on sensitive files |
| Authenticate | `tgcryptfs auth --phone <phone>` |
| Mount | `tgcryptfs mount ~/tgcryptfs --password-file <key>` |

For questions or issues, see the main [README](../README.md) or open an issue on GitHub.
