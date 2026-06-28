#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ARCH="${1:-aarch64}"

case "$ARCH" in
    aarch64|arm64)
        ARCH="aarch64"
        DOCKER_PLATFORM="linux/arm64"
        DEBIAN_IMAGE_DEFAULT="debian:trixie-slim@sha256:e9606f88b5f49b14d013d5c6d54ac7e11a48e13a6ec4c99d952330d03ddc703f"
        ;;
    x86_64|amd64)
        ARCH="x86_64"
        DOCKER_PLATFORM="linux/amd64"
        DEBIAN_IMAGE_DEFAULT="debian:trixie-slim@sha256:1275c5673a6135ff07b289ddafe4e2270dceb08eda14c0c69bb1b93ee25a9416"
        ;;
    *)
        echo "Usage: $0 [aarch64|x86_64]"
        exit 1
        ;;
esac
DEBIAN_IMAGE="${DEBIAN_IMAGE:-$DEBIAN_IMAGE_DEFAULT}"

OUT_DIR="$SCRIPT_DIR/out/$ARCH"
TARGET_VOLUME="${APC_DOCKER_TARGET_VOLUME:-apc-target-debian-$ARCH}"
mkdir -p "$OUT_DIR"

echo "==> Building apc-fuse (arch=$ARCH, image=$DEBIAN_IMAGE)"
echo "    target cache: $TARGET_VOLUME"

docker run --rm \
    --platform "$DOCKER_PLATFORM" \
    -v "$WORKSPACE_DIR:/work" \
    -v "cargo-home-debian-$ARCH:/cargo-home" \
    -v "rustup-debian-$ARCH:/rustup" \
    -v "$TARGET_VOLUME:/target" \
    -w /work \
    -e CARGO_HOME=/cargo-home \
    -e RUSTUP_HOME=/rustup \
    -e CARGO_TARGET_DIR=/target \
    "$DEBIAN_IMAGE" \
    bash -lc '
set -euxo pipefail
apt-get update
apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    libfuse3-dev \
    pkg-config

if [ ! -x /cargo-home/bin/cargo ]; then
    curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi

export PATH="$CARGO_HOME/bin:$PATH"
cargo build --release -p apc-fuse
cp /target/release/apc-fuse /work/guest/out/'"$ARCH"'/
'

echo "==> FUSE binary built: $OUT_DIR/apc-fuse"
ls -lh "$OUT_DIR/apc-fuse"
