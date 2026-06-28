#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

ARCH="$(normalize_arch "${1:-aarch64}")"
DOCKER_PLATFORM="${APC_BROWSERD_BUILD_PLATFORM:-$(docker_platform_for_arch "$ARCH")}"
DEBIAN_IMAGE="${DEBIAN_IMAGE:-debian:trixie-slim}"
JOBS="${JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 8)}"
WORKSPACE_GIT_DIR="$(git -C "$WORKSPACE_DIR" rev-parse --path-format=absolute --git-common-dir)"
CONTAINER_BROWSERD_DIR="/apc-browserd/third_party/browserd"
CIPD_PLATFORM="${APC_BROWSERD_CIPD_PLATFORM:-linux-amd64}"
FLAGS_MOUNTS=()

require_browserd_checkout

if [ "$ARCH" = "aarch64" ]; then
    FLAGS_DIR="$APC_BROWSERD_DIR/out/build-flags/$ARCH"
    mkdir -p "$FLAGS_DIR"
    cp "$BROWSERD_DIR/flags.gn" "$FLAGS_DIR/flags.gn"
    printf '\ntarget_cpu = "arm64"\nuse_siso = false\n' >> "$FLAGS_DIR/flags.gn"
    FLAGS_MOUNTS=(-v "$FLAGS_DIR/flags.gn:$CONTAINER_BROWSERD_DIR/flags.gn:ro")
fi

if [ "${APC_BROWSERD_SKIP_BUILD:-0}" != "1" ]; then
    echo "==> Building browserd (arch=$ARCH, image=$DEBIAN_IMAGE)"
    docker run --rm \
        --platform "$DOCKER_PLATFORM" \
        -v "$BROWSERD_DIR:$CONTAINER_BROWSERD_DIR" \
        -v "$WORKSPACE_GIT_DIR:/.git" \
        "${FLAGS_MOUNTS[@]}" \
        -w "$CONTAINER_BROWSERD_DIR" \
        -e JOBS="$JOBS" \
        -e APC_BROWSERD_TARGET_ARCH="$ARCH" \
        -e CONTAINER_BROWSERD_DIR="$CONTAINER_BROWSERD_DIR" \
        -e CHROMIUM_CIPD_PLATFORM="$CIPD_PLATFORM" \
        "$DEBIAN_IMAGE" \
        bash -lc '
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
extra_apt_packages=()
if [ "${APC_BROWSERD_TARGET_ARCH}" = "aarch64" ]; then
    dpkg --add-architecture amd64
    extra_apt_packages=(libc6:amd64 libstdc++6:amd64 zlib1g:amd64 libexpat1:amd64)
fi
apt-get update -qq >/tmp/apc-browserd-apt.log
apt-get install -y -qq --no-install-recommends \
    bash \
    build-essential \
    ca-certificates \
    curl \
    file \
    git \
    gperf \
    libcups2-dev \
    libdbus-1-dev \
    libdrm-dev \
    libegl1-mesa-dev \
    libgbm-dev \
    libglib2.0-dev \
    libgtk-3-dev \
    libnss3-dev \
    libpango1.0-dev \
    libpulse-dev \
    libudev-dev \
    libwayland-dev \
    libx11-dev \
    libxcb1-dev \
    libxcomposite-dev \
    libxcursor-dev \
    libxdamage-dev \
    libxext-dev \
    libxfixes-dev \
    libxi-dev \
    libxkbcommon-dev \
    libxrandr-dev \
    lsb-release \
    pkg-config \
    python3 \
    rsync \
    xz-utils \
    "${extra_apt_packages[@]}" >>/tmp/apc-browserd-apt.log

git -C /tmp config --global --add safe.directory "$CONTAINER_BROWSERD_DIR"
git -C /tmp config --global --add safe.directory "$CONTAINER_BROWSERD_DIR/chromium/src"
git -C /tmp config --global --add safe.directory "$CONTAINER_BROWSERD_DIR/chromium/depot_tools"
source scripts/lib.sh
ensure_depot_tools

if command -v gperf >/dev/null 2>&1; then
    mkdir -p "$SRC_DIR/third_party/gperf/cipd/bin"
    ln -sf "$(command -v gperf)" "$SRC_DIR/third_party/gperf/cipd/bin/gperf"
fi

if [ ! -x "$SRC_DIR/third_party/gperf/cipd/bin/gperf" ]; then
    echo "==> Installing Chromium gperf CIPD tool"
    cat >/tmp/apc-gperf.ensure <<EOF
\$ParanoidMode CheckPresence
@Subdir src/third_party/gperf/cipd
infra/3pp/tools/gperf/${CHROMIUM_CIPD_PLATFORM} version:3@3.2
EOF
    "$DEPOT_TOOLS_BOOTSTRAP/.cipd_client" ensure \
        -root "$ROOT_DIR/chromium" \
        -ensure-file /tmp/apc-gperf.ensure
fi

if [ ! -x "$SRC_DIR/third_party/gperf/cipd/bin/gperf" ]; then
    echo "==> Missing Chromium gperf CIPD tool; running gclient sync"
    cd "$ROOT_DIR/chromium"
    gclient sync -D --no-history
    cd "$ROOT_DIR"
fi

rm -f "$SRC_DIR/out/Release"/.siso*
scripts/build.sh
ninja -C "$SRC_DIR/out/Release" chrome:packed_resources
'
else
    echo "==> Skipping browserd compile; packaging existing out/Release"
fi

"$SCRIPT_DIR/package-runtime.sh" "$ARCH"
