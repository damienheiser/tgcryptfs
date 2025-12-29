#!/bin/bash
# tgcryptfs home directory sync daemon
# Syncs home directory to tgcryptfs mount with proper logging

set -euo pipefail

# Configuration
MOUNT_POINT="${TGCRYPTFS_MOUNT:-$HOME/mnt/tgcryptfs}"
BACKUP_DIR="${TGCRYPTFS_BACKUP_DIR:-$MOUNT_POINT/home}"
PASSWORD_FILE="${TGCRYPTFS_PASSWORD_FILE:-$HOME/.local/share/tgcryptfs/encryption.key}"
EXCLUDES_FILE="${TGCRYPTFS_EXCLUDES:-$HOME/.local/share/tgcryptfs/rsync-excludes.txt}"
SYNC_INTERVAL="${TGCRYPTFS_SYNC_INTERVAL:-3600}"  # Default: 1 hour
TGCRYPTFS_BIN="${TGCRYPTFS_BIN:-$(which tgcryptfs 2>/dev/null || echo "$HOME/.local/bin/tgcryptfs")}"

# Logging functions
log_info() {
    local msg="$1"
    if [[ "$OSTYPE" == "darwin"* ]]; then
        # macOS: use os_log via logger
        logger -p user.info -t tgcryptfs-sync "$msg"
    elif command -v logger &>/dev/null && journalctl --version &>/dev/null 2>&1; then
        # Linux with journald
        logger -p user.info -t tgcryptfs-sync "$msg"
    else
        # Fallback: log file
        echo "$(date '+%Y-%m-%d %H:%M:%S') [INFO] $msg" >> "${LOG_FILE:-/var/log/tgcryptfs-sync.log}"
    fi
    echo "[INFO] $msg"
}

log_error() {
    local msg="$1"
    if [[ "$OSTYPE" == "darwin"* ]]; then
        logger -p user.error -t tgcryptfs-sync "$msg"
    elif command -v logger &>/dev/null && journalctl --version &>/dev/null 2>&1; then
        logger -p user.error -t tgcryptfs-sync "$msg"
    else
        echo "$(date '+%Y-%m-%d %H:%M:%S') [ERROR] $msg" >> "${LOG_FILE:-/var/log/tgcryptfs-sync.log}"
    fi
    echo "[ERROR] $msg" >&2
}

# Check if mount is active
is_mounted() {
    mount | grep -q "tgcryptfs on $MOUNT_POINT"
}

# Mount tgcryptfs if not mounted
ensure_mounted() {
    if is_mounted; then
        log_info "tgcryptfs already mounted at $MOUNT_POINT"
        return 0
    fi

    log_info "Mounting tgcryptfs at $MOUNT_POINT"
    if [[ ! -f "$PASSWORD_FILE" ]]; then
        log_error "Password file not found: $PASSWORD_FILE"
        return 1
    fi

    mkdir -p "$MOUNT_POINT"
    "$TGCRYPTFS_BIN" mount "$MOUNT_POINT" --password-file "$PASSWORD_FILE" 2>&1 || true
    sleep 5

    if ! is_mounted; then
        log_error "Failed to mount tgcryptfs"
        return 1
    fi
    log_info "tgcryptfs mounted successfully"
}

# Run rsync
run_sync() {
    log_info "Starting sync of $HOME to $BACKUP_DIR"

    if [[ ! -f "$EXCLUDES_FILE" ]]; then
        log_error "Excludes file not found: $EXCLUDES_FILE"
        return 1
    fi

    mkdir -p "$BACKUP_DIR"

    local start_time=$(date +%s)

    if rsync -avh --delete \
        --exclude-from="$EXCLUDES_FILE" \
        "$HOME/" "$BACKUP_DIR/"; then
        local end_time=$(date +%s)
        local duration=$((end_time - start_time))
        log_info "Sync completed successfully in ${duration}s"
        return 0
    else
        log_error "Sync failed with exit code $?"
        return 1
    fi
}

# Main daemon loop
daemon_loop() {
    log_info "tgcryptfs-sync daemon starting (interval: ${SYNC_INTERVAL}s)"

    while true; do
        if ensure_mounted; then
            run_sync || true
        fi

        log_info "Sleeping for ${SYNC_INTERVAL}s until next sync"
        sleep "$SYNC_INTERVAL"
    done
}

# Single sync run
single_sync() {
    if ensure_mounted; then
        run_sync
    else
        exit 1
    fi
}

# Main
case "${1:-daemon}" in
    daemon)
        daemon_loop
        ;;
    sync|once)
        single_sync
        ;;
    status)
        if is_mounted; then
            echo "tgcryptfs is mounted at $MOUNT_POINT"
            du -sh "$BACKUP_DIR" 2>/dev/null || echo "Backup directory not found"
        else
            echo "tgcryptfs is not mounted"
            exit 1
        fi
        ;;
    *)
        echo "Usage: $0 {daemon|sync|once|status}"
        exit 1
        ;;
esac
