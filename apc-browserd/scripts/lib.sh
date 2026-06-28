#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APC_BROWSERD_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_DIR="$(cd "$APC_BROWSERD_DIR/.." && pwd)"
BROWSERD_DIR="$APC_BROWSERD_DIR/third_party/browserd"

normalize_arch() {
    case "${1:-aarch64}" in
        aarch64|arm64)
            echo "aarch64"
            ;;
        x86_64|amd64)
            echo "x86_64"
            ;;
        *)
            echo "Usage: $0 [aarch64|x86_64]" >&2
            exit 1
            ;;
    esac
}

docker_platform_for_arch() {
    case "$1" in
        aarch64)
            echo "linux/arm64"
            ;;
        x86_64)
            echo "linux/amd64"
            ;;
    esac
}

require_browserd_checkout() {
    if [ ! -e "$BROWSERD_DIR/.git" ]; then
        git -C "$WORKSPACE_DIR" submodule update --init apc-browserd/third_party/browserd
    fi
}
