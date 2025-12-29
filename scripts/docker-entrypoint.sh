#!/bin/bash
# Docker entrypoint for tgcryptfs
set -e

# Default data directory
DATA_DIR="${TGCRYPTFS_DATA_DIR:-/data}"
MOUNT_POINT="${TGCRYPTFS_MOUNT:-/mnt/tgcryptfs}"

# Ensure directories exist
mkdir -p "$DATA_DIR" "$MOUNT_POINT"

# Function to handle shutdown
cleanup() {
    echo "Shutting down tgcryptfs..."
    if mount | grep -q "tgcryptfs on $MOUNT_POINT"; then
        tgcryptfs unmount "$MOUNT_POINT" || fusermount -u "$MOUNT_POINT" || true
    fi
    exit 0
}

trap cleanup SIGTERM SIGINT

case "${1:-mount}" in
    mount)
        # Check for required files
        if [[ ! -f "$TGCRYPTFS_PASSWORD_FILE" ]]; then
            echo "ERROR: Password file not found: $TGCRYPTFS_PASSWORD_FILE"
            echo "Create it with: echo 'your-password' > $TGCRYPTFS_PASSWORD_FILE"
            exit 1
        fi

        # Check for session file
        if [[ ! -f "$DATA_DIR/pleasure.session" ]] && [[ ! -f "$DATA_DIR/tgcryptfs.session" ]]; then
            echo "WARNING: No Telegram session found. You may need to run 'auth' first."
        fi

        echo "Starting tgcryptfs mount at $MOUNT_POINT..."
        exec tgcryptfs mount "$MOUNT_POINT" \
            --password-file "$TGCRYPTFS_PASSWORD_FILE" \
            --data-dir "$DATA_DIR" \
            --foreground
        ;;

    auth)
        echo "Starting Telegram authentication..."
        exec tgcryptfs auth --data-dir "$DATA_DIR" "$@"
        ;;

    sync)
        # Run sync daemon
        export TGCRYPTFS_MOUNT="$MOUNT_POINT"
        export TGCRYPTFS_BACKUP_DIR="${TGCRYPTFS_BACKUP_DIR:-$MOUNT_POINT/backup}"
        exec /usr/local/bin/tgcryptfs-sync.sh daemon
        ;;

    shell|sh|bash)
        exec /bin/bash
        ;;

    *)
        # Pass through to tgcryptfs
        exec tgcryptfs "$@"
        ;;
esac
