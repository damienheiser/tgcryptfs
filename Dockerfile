# tgcryptfs Docker image
# Multi-stage build for minimal final image

# Build stage
FROM rust:1.83-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libfuse3-dev \
    libsqlite3-dev \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy source
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build release binary
RUN cargo build --release --locked

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    fuse3 \
    libsqlite3-0 \
    libssl3 \
    rsync \
    && rm -rf /var/lib/apt/lists/*

# Allow FUSE in container
RUN echo 'user_allow_other' >> /etc/fuse.conf

# Copy binary from builder
COPY --from=builder /build/target/release/tgcryptfs /usr/local/bin/tgcryptfs

# Copy sync script
COPY scripts/tgcryptfs-sync.sh /usr/local/bin/tgcryptfs-sync.sh
RUN chmod +x /usr/local/bin/tgcryptfs-sync.sh

# Create mount points
RUN mkdir -p /mnt/tgcryptfs /data

# Volume for persistent data (session, config, cache)
VOLUME ["/data"]

# Environment
ENV TGCRYPTFS_DATA_DIR=/data
ENV TGCRYPTFS_MOUNT=/mnt/tgcryptfs
ENV TGCRYPTFS_PASSWORD_FILE=/data/encryption.key
ENV TGCRYPTFS_EXCLUDES=/data/rsync-excludes.txt

# Health check
HEALTHCHECK --interval=60s --timeout=10s --start-period=30s --retries=3 \
    CMD mount | grep -q tgcryptfs || exit 1

# Entry point
COPY scripts/docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh
ENTRYPOINT ["/docker-entrypoint.sh"]

# Default command
CMD ["mount"]
