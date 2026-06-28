#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

ARCH="$(normalize_arch "${1:-aarch64}")"
require_browserd_checkout

SRC_OUT="$BROWSERD_DIR/chromium/src/out/Release"
DEST="$APC_BROWSERD_DIR/out/$ARCH"

if [ ! -x "$SRC_OUT/browserd" ]; then
    echo "ERROR: missing browserd binary at $SRC_OUT/browserd"
    echo "Run apc-browserd/scripts/build.sh $ARCH first."
    exit 1
fi

echo "==> Packaging browserd runtime"
echo "    source: $SRC_OUT"
echo "    output: $DEST"

rm -rf "$DEST"
mkdir -p "$DEST"

copy_file() {
    local path="$1"
    if [ -e "$SRC_OUT/$path" ]; then
        install -m 0755 "$SRC_OUT/$path" "$DEST/$path"
    fi
}

copy_data_file() {
    local path="$1"
    if [ -e "$SRC_OUT/$path" ]; then
        install -m 0644 "$SRC_OUT/$path" "$DEST/$path"
    fi
}

copy_dir() {
    local path="$1"
    if [ -d "$SRC_OUT/$path" ]; then
        mkdir -p "$DEST/$path"
        rsync -a --delete "$SRC_OUT/$path/" "$DEST/$path/"
    fi
}

copy_file browserd
copy_file chrome_crashpad_handler

for pattern in '*.so' '*.so.*' 'libEGL*' 'libGLESv2*' 'libvk_swiftshader*' 'libvulkan*'; do
    for file in "$SRC_OUT"/$pattern; do
        [ -e "$file" ] || continue
        install -m 0755 "$file" "$DEST/$(basename "$file")"
    done
done

for pattern in '*.pak' '*.bin' '*.dat' '*_icd.json'; do
    for file in "$SRC_OUT"/$pattern; do
        [ -e "$file" ] || continue
        install -m 0644 "$file" "$DEST/$(basename "$file")"
    done
done

copy_data_file product_logo_48.png

for dir in angledata hyphen-data locales MEIPreload PrivacySandboxAttestationsPreloaded resources WidevineCdm; do
    copy_dir "$dir"
done

for runtime_libs_dir in "$SRC_OUT"/*-runtime-libs; do
    [ -d "$runtime_libs_dir" ] || continue
    mkdir -p "$DEST/lib"
    rsync -a "$runtime_libs_dir/" "$DEST/lib/"
done

find "$DEST" -type d -exec chmod 0755 {} +
find "$DEST" -type f -name '*.TOC' -delete
find "$DEST" -type f -name 'ldd.txt' -delete

echo "==> browserd runtime packaged ($(du -sh "$DEST" | cut -f1))"
