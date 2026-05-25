#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ARCH="${1:-aarch64}"
DISK_SIZE_MB=4096

case "$ARCH" in
    aarch64|arm64)
        ARCH="aarch64"
        DOCKER_PLATFORM="linux/arm64"
        ;;
    x86_64|amd64)
        ARCH="x86_64"
        DOCKER_PLATFORM="linux/amd64"
        ;;
    *)
        echo "Usage: $0 [aarch64|x86_64]"
        exit 1
        ;;
esac

OUT_DIR="$SCRIPT_DIR/out/$ARCH"
mkdir -p "$OUT_DIR"

echo "==> Building AgentOS guest image (arch=$ARCH)"
echo "    output: $OUT_DIR/{vmlinuz,initramfs,disk.img}"

# Write the build script to a temp file so we avoid shell quoting issues
BUILD_SCRIPT=$(mktemp)
cat > "$BUILD_SCRIPT" << 'BUILDSCRIPT'
#!/bin/sh
set -eux

DISK_SIZE_MB="$1"

echo "--- Installing build tools ---"
apk add --no-cache e2fsprogs

echo "--- Creating rootfs ---"
mkdir -p /rootfs

# Copy APK keys so signature verification works
mkdir -p /rootfs/etc/apk/keys
cp /etc/apk/keys/* /rootfs/etc/apk/keys/

# Install Alpine into /rootfs (base packages from 3.21)
apk add --root /rootfs --initdb --no-cache \
    --repositories-file /etc/apk/repositories \
    alpine-base \
    linux-virt \
    openrc \
    busybox-openrc \
    dhcpcd \
    dbus \
    dbus-openrc \
    eudev \
    eudev-openrc \
    seatd \
    seatd-openrc \
    libseat \
    libinput \
    font-noto \
    font-noto-cjk \
    adwaita-icon-theme \
    xwayland \
    wlr-randr \
    alacritty \
    neovim \
    font-jetbrains-mono \
    bash \
    curl \
    iputils \
    sudo \
    shadow \
    util-linux \
    iproute2 \
    vulkan-tools \
    fuse3 \
    fuse3-dev

# Install all Mesa + Vulkan + wayland from Alpine Edge
# --upgrade forces replacement of 3.21 packages with Edge versions
# (Edge mesa-egl needs Edge wayland 1.25 for wl_display_dispatch_queue_timeout)
apk add --root /rootfs --no-cache --upgrade \
    --repository https://dl-cdn.alpinelinux.org/alpine/edge/main \
    --repository https://dl-cdn.alpinelinux.org/alpine/edge/community \
    wayland-libs-client \
    wayland-libs-server \
    wayland-libs-egl \
    mesa-dri-gallium \
    mesa-egl \
    mesa-gl \
    mesa-gbm \
    mesa-vulkan-virtio \
    vulkan-loader \
    chromium

echo "--- Copying overlay ---"
cp -a /overlay/* /rootfs/
chmod +x /rootfs/usr/local/bin/start-compositor

# Copy compositor binary if present
if [ -f /output/agentos-compositor ]; then
    cp /output/agentos-compositor /rootfs/usr/local/bin/
    chmod +x /rootfs/usr/local/bin/agentos-compositor
    echo "    compositor binary installed"
fi

# Copy FUSE mount binary if present
if [ -f /output/agentos-fuse ]; then
    cp /output/agentos-fuse /rootfs/usr/local/bin/
    chmod +x /rootfs/usr/local/bin/agentos-fuse
    echo "    fuse binary installed"
fi

echo "--- Configuring rootfs ---"

# Hostname
echo "agentos" > /rootfs/etc/hostname

# Root password: agentos
echo "root:agentos" | chroot /rootfs /usr/sbin/chpasswd 2>/dev/null || true

# Create agentos user with passwordless sudo
chroot /rootfs /usr/sbin/useradd -m -s /bin/bash -G wheel,video,input,seat agentos 2>/dev/null || true
echo "agentos:agentos" | chroot /rootfs /usr/sbin/chpasswd 2>/dev/null || true
mkdir -p /rootfs/etc/sudoers.d
echo "agentos ALL=(ALL) NOPASSWD: ALL" > /rootfs/etc/sudoers.d/agentos
chmod 440 /rootfs/etc/sudoers.d/agentos
chown -R 1000:1000 /rootfs/home/agentos

# iputils provides /bin/ping with cap_net_raw; ping_group_range sysctl
# set in fast-init allows unprivileged ICMP as fallback

# Fast boot — bypass OpenRC, launch compositor directly
# Use a boot script instead of many inittab lines (inittab can't do sequencing well)
cat > /rootfs/sbin/fast-init << 'FASTINIT'
#!/bin/sh
kmsg() { echo "$1" > /dev/kmsg 2>/dev/null; }

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts
mount -t devpts devpts /dev/pts -o gid=5,mode=620,ptmxmode=666
mount -o remount,rw /
mount -t tmpfs tmpfs /tmp
mount -t tmpfs tmpfs /run
mkdir -p /dev/shm
mount -t tmpfs tmpfs /dev/shm -o mode=1777
kmsg "fast-init: mounts done"

# mke2fs -d strips suid bits, so restore at boot
[ -e /bin/bbsuid ] && chmod u+s /bin/bbsuid 2>/dev/null

# Allow unprivileged ICMP (ping without suid/capabilities)
echo "0 65534" > /proc/sys/net/ipv4/ping_group_range 2>/dev/null

# Start eudev early so it sees module load events and populates udev db
udevd --daemon 2>/dev/null
kmsg "fast-init: udevd started"

# Load modules — udevd will process the resulting device events
KVER=$(uname -r)
depmod -a "$KVER" 2>/dev/null
modprobe virtio-gpu &
modprobe virtio_input
modprobe evdev
modprobe vsock
modprobe virtio_transport
modprobe vmw_vsock_virtio_transport
modprobe fuse
kmsg "fast-init: modprobe done"

# Wait for DRM device to appear
for i in $(seq 1 40); do
    [ -d /sys/class/drm/card0 ] && break
    sleep 0.25
done

if [ -d /sys/class/drm/card0 ]; then
    kmsg "fast-init: card0 in sysfs"
    # Ensure render node exists
    if [ ! -e /dev/dri/renderD128 ] && [ -e /sys/class/drm/renderD128/dev ]; then
        DEVNUM=$(cat /sys/class/drm/renderD128/dev)
        MAJOR=${DEVNUM%%:*}
        MINOR=${DEVNUM##*:}
        mkdir -p /dev/dri
        mknod /dev/dri/renderD128 c "$MAJOR" "$MINOR"
        chmod 666 /dev/dri/renderD128
    fi
    # Make DRI devices accessible to video group
    chgrp video /dev/dri/* 2>/dev/null
    chmod 660 /dev/dri/* 2>/dev/null
    chmod 666 /dev/dri/renderD128 2>/dev/null
    kmsg "fast-init: DRM ready ($(ls /dev/dri/ 2>/dev/null | tr '\n' ' '))"
else
    kmsg "fast-init: WARNING no card0 after 10s"
fi

# Trigger udev to populate db for all existing devices
udevadm trigger --action=add 2>/dev/null
udevadm settle --timeout=5 2>/dev/null

# Ensure /dev/input nodes exist (udevd should create them, but fallback)
mkdir -p /dev/input
for ev in /sys/class/input/event*; do
    [ -e "$ev/dev" ] || continue
    name=$(basename "$ev")
    if [ ! -e "/dev/input/$name" ]; then
        DEVNUM=$(cat "$ev/dev")
        MAJOR=${DEVNUM%%:*}
        MINOR=${DEVNUM##*:}
        mknod "/dev/input/$name" c "$MAJOR" "$MINOR"
        chmod 666 "/dev/input/$name"
    fi
done
# Make input devices accessible to input group
chgrp input /dev/input/* 2>/dev/null
chmod 660 /dev/input/* 2>/dev/null
kmsg "fast-init: input devices ($(ls /dev/input/ 2>/dev/null | tr '\n' ' '))"

# Hostname
hostname agentos

# Start dbus system bus
mkdir -p /run/dbus
dbus-daemon --system 2>/dev/null
kmsg "fast-init: dbus started"

# Networking: virtio-net via libslirp (userspace NAT on host)
# libkrun provides a virtio-net device backed by a socketpair.
# Guest gets a real eth0 interface with DHCP from slirp (10.0.2.0/24).
ip link set lo up 2>/dev/null
modprobe virtio_net 2>/dev/null

# Wait for eth0 to appear
for i in $(seq 1 20); do
    [ -e /sys/class/net/eth0 ] && break
    sleep 0.25
done

if [ -e /sys/class/net/eth0 ]; then
    ip link set eth0 up 2>/dev/null
    dhcpcd -w --timeout 10 eth0 2>/dev/null &
    kmsg "fast-init: eth0 up, dhcpcd started"
else
    kmsg "fast-init: WARNING no eth0 found"
fi

# Network diagnostics
kmsg "netdiag: ifaces=$(ip -o addr 2>&1 | tr '\n' '|')"
kmsg "netdiag: routes=$(ip route 2>&1 | tr '\n' '|')"
kmsg "netdiag: vsock=$(ls /dev/vsock 2>&1)"

kmsg "fast-init: complete"
FASTINIT
chmod +x /rootfs/sbin/fast-init

cat > /rootfs/etc/inittab << 'EOF'
::sysinit:/sbin/fast-init
::respawn:/usr/local/bin/start-compositor
EOF

# Keep OpenRC inittab as fallback
cat > /rootfs/etc/inittab.openrc << 'EOF'
::sysinit:/sbin/openrc sysinit
::sysinit:/sbin/openrc boot
::wait:/sbin/openrc default
::respawn:/usr/local/bin/start-compositor
::shutdown:/sbin/openrc shutdown
EOF

# Filesystem table
cat > /rootfs/etc/fstab << 'EOF'
/dev/vda    /           ext4    rw,relatime     0 1
proc        /proc       proc    defaults        0 0
sysfs       /sys        sysfs   defaults        0 0
devtmpfs    /dev        devtmpfs defaults       0 0
tmpfs       /tmp        tmpfs   defaults,nosuid 0 0
tmpfs       /run        tmpfs   defaults,nosuid 0 0
shared      /mnt/shared virtiofs defaults,nofail 0 0
EOF

# Network
cat > /rootfs/etc/network/interfaces << 'EOF'
auto lo
iface lo inet loopback

auto eth0
iface eth0 inet dhcp
EOF

# Kernel modules
cat > /rootfs/etc/modules << 'EOF'
virtio_gpu
virtio_net
virtio_blk
virtio_console
virtiofs
vsock
virtio_transport
vmw_vsock_virtio_transport
fuse
dummy
EOF

# Chromium: use Wayland, no sandbox (running as root in VM)
mkdir -p /rootfs/etc/chromium
cat > /rootfs/etc/chromium/chromium.conf << 'EOF'
CHROMIUM_FLAGS="--ozone-platform=wayland --disable-breakpad --disable-crash-reporter --enable-features=UseOzonePlatform --ignore-gpu-blocklist --enable-gpu-rasterization --enable-zero-copy --disable-infobars --user-data-dir=/home/agentos/.config/chromium"
EOF

# Enable services
ln -sf /etc/init.d/udev         /rootfs/etc/runlevels/sysinit/udev
ln -sf /etc/init.d/udev-trigger /rootfs/etc/runlevels/sysinit/udev-trigger
ln -sf /etc/init.d/udev-settle  /rootfs/etc/runlevels/sysinit/udev-settle
ln -sf /etc/init.d/seatd        /rootfs/etc/runlevels/boot/seatd
ln -sf /etc/init.d/hostname     /rootfs/etc/runlevels/boot/hostname

# Misc
mkdir -p /rootfs/mnt/shared
echo "nameserver 8.8.8.8" > /rootfs/etc/resolv.conf

echo "--- Patching Alpine initramfs for virtio-mmio ---"
# Alpine's nlplug-findfs blocks on virtio-mmio because it waits for uevents.
# Patch the init to skip nlplug-findfs and mount root directly.
INITRAMFS_DIR=$(mktemp -d)
cd "$INITRAMFS_DIR"
gzip -dc /rootfs/boot/initramfs-virt | cpio -idm 2>/dev/null

# Replace nlplug-findfs with a script that just runs mdev and returns
cat > "$INITRAMFS_DIR/sbin/nlplug-findfs" << 'NLPLUG'
#!/bin/sh
# Stub: run mdev to populate /dev, then return immediately
/sbin/mdev -s 2>/dev/null
NLPLUG
chmod +x "$INITRAMFS_DIR/sbin/nlplug-findfs"

# Repack
(find . | cpio -o -H newc 2>/dev/null | gzip -9) > /output/initramfs
echo "    initramfs: patched Alpine ($(du -h /output/initramfs | cut -f1))"
cd /
rm -rf "$INITRAMFS_DIR"

# Remove kernel from disk (host loads directly)
rm -f /rootfs/boot/vmlinuz-* /rootfs/boot/initramfs-*

echo "--- Creating disk image (${DISK_SIZE_MB}MB) ---"
truncate -s "${DISK_SIZE_MB}M" /tmp/disk.img
mke2fs -t ext4 -L agentos -F /tmp/disk.img
MOUNT_DIR=$(mktemp -d)
mount -o loop /tmp/disk.img "$MOUNT_DIR"
cp -a /rootfs/* "$MOUNT_DIR/"
sync
umount "$MOUNT_DIR"
rmdir "$MOUNT_DIR"
cp /tmp/disk.img /output/disk.img
rm /tmp/disk.img

echo "--- Done ---"
ls -lh /output/
BUILDSCRIPT

docker run --rm --privileged \
    --platform "$DOCKER_PLATFORM" \
    -v "$SCRIPT_DIR/rootfs:/overlay:ro" \
    -v "$OUT_DIR:/output" \
    -v "$BUILD_SCRIPT:/build.sh:ro" \
    alpine:3.21 \
    sh /build.sh "$DISK_SIZE_MB"

rm -f "$BUILD_SCRIPT"

# Copy TSI-patched kernel from libkrunfw build (host-side, not in Docker)
# libkrun requires a raw ARM64 Image with TSI patches — Alpine's PE/EFI kernel won't boot
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
KERNEL_IMAGE=$(find "$WORKSPACE_DIR/deps/src/libkrunfw" -path "*/arch/arm64/boot/Image" 2>/dev/null | head -1)

if [ -n "$KERNEL_IMAGE" ]; then
    cp "$KERNEL_IMAGE" "$OUT_DIR/vmlinuz"
    echo "==> Kernel: copied from libkrunfw ($(du -h "$OUT_DIR/vmlinuz" | cut -f1))"
elif [ -f "$OUT_DIR/vmlinuz" ]; then
    echo "==> Kernel: using existing $OUT_DIR/vmlinuz (libkrunfw source not found)"
else
    echo "ERROR: No kernel available. Run deps/build-deps.sh first to build libkrunfw."
    exit 1
fi

echo ""
echo "==> Guest image built successfully"
echo "    Kernel:    $OUT_DIR/vmlinuz"
echo "    Initramfs: $OUT_DIR/initramfs"
echo "    Disk:      $OUT_DIR/disk.img"
echo ""
echo "    Run with:"
echo "    cargo run -p agentos-host -- \\"
echo "      --kernel $OUT_DIR/vmlinuz \\"
echo "      --initrd $OUT_DIR/initramfs \\"
echo "      --disk $OUT_DIR/disk.img"
