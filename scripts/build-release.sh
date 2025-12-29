#!/bin/bash
# Build tgcryptfs for multiple platforms
# Requires: cross (cargo install cross), Docker

set -euo pipefail

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)}"
DIST_DIR="dist"
PROJECT_NAME="tgcryptfs"

echo "Building $PROJECT_NAME v$VERSION"
echo "================================"

# Create dist directory
mkdir -p "$DIST_DIR"

# Define targets
# Format: "target:binary_suffix:cross_or_native"
TARGETS=(
    # macOS (native only - can't cross-compile to macOS)
    "aarch64-apple-darwin:macos-aarch64:native"
    "x86_64-apple-darwin:macos-x86_64:native"

    # Linux x86_64 (glibc and musl)
    "x86_64-unknown-linux-gnu:linux-x86_64:cross"
    "x86_64-unknown-linux-musl:linux-x86_64-musl:cross"

    # Linux ARM64/aarch64 (Asahi Linux, Raspberry Pi 4, ARM servers)
    "aarch64-unknown-linux-gnu:linux-aarch64:cross"
    "aarch64-unknown-linux-musl:linux-aarch64-musl:cross"

    # Linux ARMv7 (Raspberry Pi 2/3, older ARM)
    "armv7-unknown-linux-gnueabihf:linux-armv7:cross"
    "armv7-unknown-linux-musleabihf:linux-armv7-musl:cross"

    # Linux MIPS (routers, embedded)
    "mips-unknown-linux-gnu:linux-mips:cross"
    "mipsel-unknown-linux-gnu:linux-mipsel:cross"
    "mips64-unknown-linux-gnuabi64:linux-mips64:cross"
    "mips64el-unknown-linux-gnuabi64:linux-mips64el:cross"

    # Linux RISC-V
    "riscv64gc-unknown-linux-gnu:linux-riscv64:cross"

    # Linux PowerPC
    "powerpc64le-unknown-linux-gnu:linux-ppc64le:cross"

    # FreeBSD (via cross)
    # "x86_64-unknown-freebsd:freebsd-x86_64:cross"

    # Android (experimental)
    # "aarch64-linux-android:android-aarch64:cross"
)

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

build_target() {
    local target="$1"
    local suffix="$2"
    local method="$3"
    local binary_name="${PROJECT_NAME}-${suffix}"

    echo -e "\n${YELLOW}Building for $target...${NC}"

    # Check if this is a macOS target on non-macOS host
    if [[ "$target" == *"apple-darwin"* ]] && [[ "$(uname)" != "Darwin" ]]; then
        echo -e "${RED}  Skipping $target (can only build on macOS)${NC}"
        return 0
    fi

    # Check if this is a non-macOS target on macOS
    if [[ "$target" != *"apple-darwin"* ]] && [[ "$(uname)" == "Darwin" ]] && [[ "$method" == "native" ]]; then
        method="cross"
    fi

    local build_cmd
    local output_path

    if [[ "$method" == "native" ]]; then
        # Native build
        if ! rustup target list --installed | grep -q "$target"; then
            echo "  Adding target $target..."
            rustup target add "$target" || {
                echo -e "${RED}  Failed to add target $target${NC}"
                return 1
            }
        fi
        build_cmd="cargo build --release --target $target"
        output_path="target/$target/release/$PROJECT_NAME"
    else
        # Cross compilation
        if ! command -v cross &>/dev/null; then
            echo -e "${RED}  'cross' not found. Install with: cargo install cross${NC}"
            return 1
        fi
        build_cmd="CROSS_CUSTOM_TOOLCHAIN=1 cross build --release --target $target"
        output_path="target/$target/release/$PROJECT_NAME"
    fi

    # Build
    if eval "$build_cmd" 2>&1; then
        if [[ -f "$output_path" ]]; then
            cp "$output_path" "$DIST_DIR/$binary_name"
            chmod +x "$DIST_DIR/$binary_name"

            # Get size
            local size=$(du -h "$DIST_DIR/$binary_name" | cut -f1)
            echo -e "${GREEN}  Built $binary_name ($size)${NC}"
        else
            echo -e "${RED}  Binary not found at $output_path${NC}"
            return 1
        fi
    else
        echo -e "${RED}  Build failed for $target${NC}"
        return 1
    fi
}

# Build for native platform first
echo "Building native release..."
cargo build --release
NATIVE_TARGET=$(rustc -vV | grep host | cut -d' ' -f2)
cp "target/release/$PROJECT_NAME" "$DIST_DIR/${PROJECT_NAME}-native"

# Build for each target
FAILED_TARGETS=()
for target_spec in "${TARGETS[@]}"; do
    IFS=':' read -r target suffix method <<< "$target_spec"

    # Skip current platform's duplicate
    if [[ "$target" == "$NATIVE_TARGET" ]]; then
        cp "$DIST_DIR/${PROJECT_NAME}-native" "$DIST_DIR/${PROJECT_NAME}-${suffix}"
        echo -e "${GREEN}Built ${PROJECT_NAME}-${suffix} (native)${NC}"
        continue
    fi

    if ! build_target "$target" "$suffix" "$method"; then
        FAILED_TARGETS+=("$target")
    fi
done

# Generate checksums
echo -e "\n${YELLOW}Generating checksums...${NC}"
cd "$DIST_DIR"
sha256sum * > SHA256SUMS 2>/dev/null || shasum -a 256 * > SHA256SUMS
cd ..

# Summary
echo -e "\n================================"
echo "Build Summary"
echo "================================"
echo "Distribution directory: $DIST_DIR/"
ls -lah "$DIST_DIR/"

if [[ ${#FAILED_TARGETS[@]} -gt 0 ]]; then
    echo -e "\n${RED}Failed targets:${NC}"
    for t in "${FAILED_TARGETS[@]}"; do
        echo "  - $t"
    done
fi

echo -e "\n${GREEN}Done!${NC}"
