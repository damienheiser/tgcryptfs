#!/bin/bash
# Build tgcryptfs for all supported platforms

set -e

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)}"
OUTPUT_DIR="dist/v${VERSION}"

echo "Building tgcryptfs v${VERSION} for all platforms..."

mkdir -p "$OUTPUT_DIR"

# macOS builds (native on macOS, cross-compile on Linux)
if [[ "$(uname)" == "Darwin" ]]; then
    echo "=== Building macOS aarch64 (native) ==="
    cargo build --release --target aarch64-apple-darwin
    cp target/aarch64-apple-darwin/release/tgcryptfs "$OUTPUT_DIR/tgcryptfs-macos-aarch64"

    echo "=== Building macOS x86_64 ==="
    rustup target add x86_64-apple-darwin 2>/dev/null || true
    cargo build --release --target x86_64-apple-darwin
    cp target/x86_64-apple-darwin/release/tgcryptfs "$OUTPUT_DIR/tgcryptfs-macos-x86_64"
fi

# Linux builds using cross (requires Docker)
if command -v cross &> /dev/null || command -v ~/.cargo/bin/cross &> /dev/null; then
    CROSS="${HOME}/.cargo/bin/cross"

    echo "=== Building Linux x86_64 ==="
    CROSS_CUSTOM_TOOLCHAIN=1 $CROSS build --release --target x86_64-unknown-linux-gnu || \
        cargo build --release --target x86_64-unknown-linux-gnu
    cp target/x86_64-unknown-linux-gnu/release/tgcryptfs "$OUTPUT_DIR/tgcryptfs-linux-x86_64" 2>/dev/null || true

    echo "=== Building Linux aarch64 ==="
    CROSS_CUSTOM_TOOLCHAIN=1 $CROSS build --release --target aarch64-unknown-linux-gnu || \
        echo "aarch64-linux cross-compile failed, skipping"
fi

# Native Linux build
if [[ "$(uname)" == "Linux" ]]; then
    echo "=== Building Linux native ==="
    cargo build --release
    ARCH=$(uname -m)
    cp target/release/tgcryptfs "$OUTPUT_DIR/tgcryptfs-linux-${ARCH}"
fi

# Generate checksums
cd "$OUTPUT_DIR"
echo "=== Generating checksums ==="
shasum -a 256 tgcryptfs-* > SHA256SUMS 2>/dev/null || sha256sum tgcryptfs-* > SHA256SUMS
cat SHA256SUMS

echo ""
echo "=== Build complete ==="
ls -la
