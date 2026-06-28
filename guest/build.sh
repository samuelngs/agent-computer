#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ARCH="${1:-aarch64}"
DEBIAN_SUITE="${DEBIAN_SUITE:-trixie}"
DEBIAN_MIRROR="${DEBIAN_MIRROR:-http://deb.debian.org/debian}"
DISK_SIZE_MB="${DISK_SIZE_MB:-4096}"
APC_BROWSERD="${APC_BROWSERD:-1}"

case "$ARCH" in
    aarch64|arm64)
        ARCH="aarch64"
        DEBIAN_ARCH="arm64"
        DOCKER_PLATFORM="linux/arm64"
        DEBIAN_IMAGE_DEFAULT="debian:${DEBIAN_SUITE}-slim@sha256:e9606f88b5f49b14d013d5c6d54ac7e11a48e13a6ec4c99d952330d03ddc703f"
        ;;
    x86_64|amd64)
        ARCH="x86_64"
        DEBIAN_ARCH="amd64"
        DOCKER_PLATFORM="linux/amd64"
        DEBIAN_IMAGE_DEFAULT="debian:${DEBIAN_SUITE}-slim@sha256:1275c5673a6135ff07b289ddafe4e2270dceb08eda14c0c69bb1b93ee25a9416"
        ;;
    *)
        echo "Usage: $0 [aarch64|x86_64]"
        exit 1
        ;;
esac
DEBIAN_IMAGE="${DEBIAN_IMAGE:-$DEBIAN_IMAGE_DEFAULT}"

OUT_DIR="$SCRIPT_DIR/out/$ARCH"
COMPOSITOR_BIN="$OUT_DIR/apc-compositor"
FUSE_BIN="$OUT_DIR/apc-fuse"
BROWSERD_RUNTIME_DIR="$WORKSPACE_DIR/apc-browserd/out/$ARCH"

if [ ! -x "$COMPOSITOR_BIN" ]; then
    echo "ERROR: missing required compositor binary: $COMPOSITOR_BIN"
    echo "Run ./guest/build-compositor.sh $ARCH first."
    exit 1
fi

if [ ! -x "$FUSE_BIN" ]; then
    echo "ERROR: missing required FUSE binary: $FUSE_BIN"
    echo "Run ./guest/build-fuse.sh $ARCH first."
    exit 1
fi

BROWSERD_DOCKER_ARGS=()
if [ "$APC_BROWSERD" = "1" ]; then
    if [ ! -x "$BROWSERD_RUNTIME_DIR/browserd" ]; then
        echo "ERROR: missing required browserd runtime: $BROWSERD_RUNTIME_DIR/browserd"
        echo "Run ./apc-browserd/scripts/build.sh $ARCH first."
        exit 1
    fi
    BROWSERD_DOCKER_ARGS=(-v "$BROWSERD_RUNTIME_DIR:/browserd-runtime:ro")
fi

mkdir -p "$OUT_DIR"

echo "==> Building APC Debian guest image"
echo "    arch:   $ARCH ($DEBIAN_ARCH)"
echo "    suite:  $DEBIAN_SUITE"
echo "    image:  $DEBIAN_IMAGE"
echo "    browserd: $APC_BROWSERD"
echo "    output: $OUT_DIR/{vmlinuz,initramfs,disk.img}"

BUILD_SCRIPT=$(mktemp)
cat > "$BUILD_SCRIPT" << 'BUILDSCRIPT'
#!/usr/bin/env bash
set -euo pipefail

DISK_SIZE_MB="$1"
DEBIAN_SUITE="$2"
DEBIAN_ARCH="$3"
DEBIAN_MIRROR="$4"
BROWSERD_ENABLED="$5"

RUNTIME_PACKAGES=(
    base-files
    base-passwd
    bash
    busybox-static
    ca-certificates
    coreutils
    curl
    dbus
    dbus-user-session
    dhcpcd-base
    findutils
    fontconfig
    foot
    fonts-noto
    fonts-noto-cjk
    fuse3
    grep
    iproute2
    iputils-ping
    kmod
    libatk-bridge2.0-0t64
    libasound2t64
    libcups2t64
    libc6
    libdrm2
    libegl1
    libgbm1
    libglib2.0-0t64
    libgles2
    libgtk-3-0t64
    libinput-bin
    libinput-tools
    libinput10
    libnspr4
    libnss3
    libpango-1.0-0
    libpangocairo-1.0-0
    libpulse0
    libseat1
    libudev1
    libwayland-client0
    libwayland-egl1
    libwayland-server0
    libx11-6
    libxcb1
    libxcomposite1
    libxcursor1
    libxdamage1
    libxext6
    libxfixes3
    libxi6
    libxkbcommon0
    libxrandr2
    libxrender1
    libxshmfence1
    mesa-utils
    mesa-vulkan-drivers
    neovim
    passwd
    procps
    seatd
    sudo
    udev
    util-linux
    wayland-protocols
    xwayland
    zlib1g
)
RUNTIME_PACKAGE_CSV="$(IFS=,; echo "${RUNTIME_PACKAGES[*]}")"

echo "--- Installing rootfs build tools ---"
apt-get update
apt-get install -y --no-install-recommends \
    ca-certificates \
    cpio \
    curl \
    debootstrap \
    e2fsprogs \
    findutils \
    gzip

echo "--- Validating Debian runtime package set ---"
apt-get install -s --no-install-recommends "${RUNTIME_PACKAGES[@]}" >/tmp/apc-apt-sim.log

echo "--- Creating Debian rootfs ---"
rm -rf /rootfs
mkdir -p /rootfs
debootstrap \
    --arch="$DEBIAN_ARCH" \
    --variant=minbase \
    --include="$RUNTIME_PACKAGE_CSV" \
    "$DEBIAN_SUITE" \
    /rootfs \
    "$DEBIAN_MIRROR"

echo "--- Copying APC overlay and binaries ---"
cp -a /overlay/. /rootfs/
install -m 0755 /output/apc-compositor /rootfs/usr/local/bin/apc-compositor
install -m 0755 /output/apc-fuse /rootfs/usr/local/bin/apc-fuse
chmod 0755 /rootfs/usr/local/bin/start-compositor
if [ "$BROWSERD_ENABLED" = "1" ]; then
    mkdir -p /rootfs/opt/apc-browserd
    cp -a /browserd-runtime/. /rootfs/opt/apc-browserd/
    chmod 0755 /rootfs/opt/apc-browserd/browserd
fi

echo "--- Configuring Debian rootfs ---"
echo "apc" > /rootfs/etc/hostname
cat > /rootfs/etc/hosts << 'EOF'
127.0.0.1 localhost
127.0.1.1 apc
::1 localhost ip6-localhost ip6-loopback
ff02::1 ip6-allnodes
ff02::2 ip6-allrouters
EOF

cat > /rootfs/etc/fstab << 'EOF'
/dev/vda    /           ext4    rw,relatime          0 1
proc        /proc       proc    defaults             0 0
sysfs       /sys        sysfs   defaults             0 0
devtmpfs    /dev        devtmpfs defaults            0 0
tmpfs       /tmp        tmpfs   defaults,nosuid      0 0
tmpfs       /run        tmpfs   defaults,nosuid      0 0
tmpfs       /dev/shm    tmpfs   defaults,nosuid,nodev 0 0
EOF
ln -sf /proc/mounts /rootfs/etc/mtab
grep -q '^user_allow_other$' /rootfs/etc/fuse.conf 2>/dev/null || echo 'user_allow_other' >> /rootfs/etc/fuse.conf

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
virtio_input
evdev
EOF

cat > /rootfs/etc/resolv.conf << 'EOF'
nameserver 10.0.2.3
nameserver 1.1.1.1
EOF

chroot /rootfs /usr/sbin/groupadd -g 1000 agent
for group in video input render seat fuse; do
    chroot /rootfs /usr/bin/getent group "$group" >/dev/null || chroot /rootfs /usr/sbin/groupadd "$group"
done
chroot /rootfs /usr/sbin/useradd \
    --uid 1000 \
    --gid 1000 \
    --create-home \
    --home-dir /home/agent \
    --shell /bin/bash \
    --groups sudo,video,input,render,seat,fuse \
    agent
echo "agent:agent" | chroot /rootfs /usr/sbin/chpasswd
mkdir -p /rootfs/etc/sudoers.d
echo "agent ALL=(ALL) NOPASSWD: ALL" > /rootfs/etc/sudoers.d/agent
chmod 0440 /rootfs/etc/sudoers.d/agent

mkdir -p /rootfs/home/agent /rootfs/mnt/shared /rootfs/run/user/1000 /rootfs/tmp /rootfs/dev/shm
if [ "$BROWSERD_ENABLED" = "1" ]; then
    mkdir -p /rootfs/home/agent/.config/apc-browserd
fi
chown -R 1000:1000 /rootfs/home/agent /rootfs/mnt/shared /rootfs/run/user/1000
chmod 0700 /rootfs/run/user/1000
chmod 1777 /rootfs/tmp /rootfs/dev/shm

cat > /rootfs/sbin/fast-init << 'FASTINIT'
#!/bin/sh
PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

kmsg() { echo "$1" > /dev/kmsg 2>/dev/null || true; }

mountpoint -q /proc || mount -t proc proc /proc
mountpoint -q /sys || mount -t sysfs sysfs /sys
mountpoint -q /dev || mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts /dev/shm /tmp /run /run/user/1000
mountpoint -q /dev/pts || mount -t devpts devpts /dev/pts -o gid=5,mode=620,ptmxmode=666
mount -o remount,rw / 2>/dev/null || true
mountpoint -q /tmp || mount -t tmpfs tmpfs /tmp -o mode=1777,nosuid,nodev
mountpoint -q /run || mount -t tmpfs tmpfs /run -o mode=0755,nosuid,nodev
mkdir -p /run/user/1000 /run/dbus /dev/shm
mountpoint -q /dev/shm || mount -t tmpfs tmpfs /dev/shm -o mode=1777,nosuid,nodev
chown 1000:1000 /run/user/1000 2>/dev/null || true
chmod 0700 /run/user/1000 2>/dev/null || true
kmsg "fast-init: mounts done"

hostname apc 2>/dev/null || true
echo "0 65534" > /proc/sys/net/ipv4/ping_group_range 2>/dev/null || true
ln -sf /proc/mounts /etc/mtab 2>/dev/null || true

# mke2fs -d can strip or normalize suid bits on some hosts. Restore only Debian
# tools that need privilege for the development workflow.
for path in /usr/bin/sudo /usr/bin/su /usr/bin/passwd /usr/bin/fusermount3; do
    [ -e "$path" ] && chmod u+s "$path" 2>/dev/null || true
done

dbus-uuidgen --ensure=/etc/machine-id 2>/dev/null || true
mkdir -p /run/udev /run/udev/rules.d

udevd_running() {
    ps -ef 2>/dev/null | grep -q '[s]ystemd-udevd'
}

start_udevd() {
    if udevd_running; then
        return 0
    fi
    rm -f /run/udev/control 2>/dev/null || true

    UDEVD=
    for candidate in /lib/systemd/systemd-udevd /usr/lib/systemd/systemd-udevd; do
        if [ -x "$candidate" ]; then
            UDEVD="$candidate"
            break
        fi
    done
    if [ -z "$UDEVD" ]; then
        kmsg "fast-init: WARNING no Debian udevd found"
        return 1
    fi

    : > /tmp/udevd.log
    "$UDEVD" >>/tmp/udevd.log 2>&1 &
    echo "$!" > /run/udev/apc-udevd.pid
    for _ in $(seq 1 300); do
        if udevadm control --ping >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.05
    done

    kmsg "fast-init: WARNING udevd failed to stay running ($(tr '\n' '|' < /tmp/udevd.log | cut -c1-300))"
    return 1
}

depmod -a "$(uname -r)" 2>/dev/null || true
for module in virtio_gpu virtio_net virtio_blk virtio_console virtiofs vsock virtio_transport vmw_vsock_virtio_transport fuse virtio_input evdev; do
    modprobe "$module" 2>/dev/null || true
done
kmsg "fast-init: modprobe done"

if start_udevd; then
    kmsg "fast-init: udevd started"
else
    sleep 0.5
    if start_udevd; then
        kmsg "fast-init: udevd started after retry"
    else
        kmsg "fast-init: WARNING continuing without running udevd"
    fi
fi

apc_trigger_udev_input() {
    udevadm trigger --action=add --subsystem-match=input 2>/dev/null || true
    udevadm trigger --action=change --subsystem-match=input 2>/dev/null || true
    udevadm settle --timeout=10 2>/dev/null || true
}

udevadm trigger --action=add 2>/dev/null || true
apc_trigger_udev_input
kmsg "fast-init: udev settle complete"

if [ -e /sys/class/misc/fuse/dev ] && [ ! -e /dev/fuse ]; then
    devnum="$(cat /sys/class/misc/fuse/dev)"
    major="${devnum%%:*}"
    minor="${devnum##*:}"
    mknod /dev/fuse c "$major" "$minor" 2>/dev/null || true
fi
if [ -e /dev/fuse ]; then
    chgrp fuse /dev/fuse 2>/dev/null || chgrp agent /dev/fuse 2>/dev/null || true
    chmod 660 /dev/fuse 2>/dev/null || true
    kmsg "fast-init: FUSE ready"
else
    kmsg "fast-init: WARNING no /dev/fuse"
fi

mkdir -p /dev/dri
for node in card0 renderD128; do
    sys="/sys/class/drm/$node/dev"
    if [ -e "$sys" ] && [ ! -e "/dev/dri/$node" ]; then
        devnum="$(cat "$sys")"
        major="${devnum%%:*}"
        minor="${devnum##*:}"
        mknod "/dev/dri/$node" c "$major" "$minor" 2>/dev/null || true
    fi
done

for i in $(seq 1 40); do
    [ -e /dev/dri/card0 ] && [ -e /dev/dri/renderD128 ] && break
    sleep 0.25
done

if [ -e /dev/dri/card0 ]; then
    chgrp video /dev/dri/card* 2>/dev/null || true
    chmod 660 /dev/dri/card* 2>/dev/null || true
    chgrp render /dev/dri/renderD* 2>/dev/null || chgrp video /dev/dri/renderD* 2>/dev/null || true
    chmod 660 /dev/dri/renderD* 2>/dev/null || true
    kmsg "fast-init: DRM ready ($(ls /dev/dri 2>/dev/null | tr '\n' ' '))"
else
    kmsg "fast-init: WARNING no DRM card0 after 10s"
fi

mkdir -p /dev/input
for ev in /sys/class/input/event*; do
    [ -e "$ev/dev" ] || continue
    name="$(basename "$ev")"
    if [ ! -e "/dev/input/$name" ]; then
        devnum="$(cat "$ev/dev")"
        major="${devnum%%:*}"
        minor="${devnum##*:}"
        mknod "/dev/input/$name" c "$major" "$minor" 2>/dev/null || true
    fi
done
chgrp input /dev/input/event* 2>/dev/null || true
chmod 660 /dev/input/event* 2>/dev/null || true
kmsg "fast-init: input devices ($(ls /dev/input 2>/dev/null | tr '\n' ' '))"

apc_input_rules_needed() {
    for dev in /dev/input/event*; do
        [ -e "$dev" ] || continue
        event_name="$(basename "$dev")"
        device_name="$(cat "/sys/class/input/$event_name/device/name" 2>/dev/null || true)"
        props="$(udevadm info -q property -n "$dev" 2>/dev/null || true)"
        case "$device_name" in
            "APC Virtual Keyboard")
                echo "$props" | grep -qx 'ID_INPUT_KEYBOARD=1' || return 0
                ;;
            "APC Virtual Pointer")
                echo "$props" | grep -qx 'ID_INPUT_MOUSE=1' || return 0
                ;;
        esac
    done
    return 1
}

if apc_input_rules_needed; then
    mkdir -p /run/udev/rules.d
    cat > /run/udev/rules.d/90-apc-input.rules << 'UDEVRULES'
ACTION=="add|change", SUBSYSTEM=="input", KERNEL=="event[0-9]*", ATTRS{name}=="APC Virtual Keyboard", ENV{ID_INPUT}="1", ENV{ID_INPUT_KEYBOARD}="1", ENV{ID_SEAT}="seat0"
ACTION=="add|change", SUBSYSTEM=="input", KERNEL=="event[0-9]*", ATTRS{name}=="APC Virtual Pointer", ENV{ID_INPUT}="1", ENV{ID_INPUT_MOUSE}="1", ENV{ID_SEAT}="seat0"
UDEVRULES
    udevadm control --reload 2>/dev/null || true
    apc_trigger_udev_input
    kmsg "fast-init: applied APC input udev fallback rules"
else
    kmsg "fast-init: native udev input classification present"
fi

for dev in /dev/input/event*; do
    [ -e "$dev" ] || continue
    event_name="$(basename "$dev")"
    device_name="$(cat "/sys/class/input/$event_name/device/name" 2>/dev/null || true)"
    props="$(udevadm info -q property -n "$dev" 2>/dev/null | tr '\n' ' ')"
    kmsg "inputdiag: $dev name=\"$device_name\" $props"
    devnum="$(cat "/sys/class/input/$event_name/dev" 2>/dev/null || true)"
    if [ -n "$devnum" ] && [ -e "/run/udev/data/c$devnum" ]; then
        kmsg "inputdiag: $dev udev_data $(tr '\n' '|' < "/run/udev/data/c$devnum" | cut -c1-700)"
    else
        kmsg "inputdiag: WARNING $dev missing udev database c$devnum"
    fi
done
if command -v libinput >/dev/null 2>&1; then
    libinput list-devices >/tmp/libinput-list-devices.log 2>&1 || true
    kmsg "inputdiag: libinput list-devices $(tr '\n' '|' < /tmp/libinput-list-devices.log)"
else
    kmsg "inputdiag: WARNING libinput CLI not installed"
fi

dbus-daemon --system 2>/dev/null || true
kmsg "fast-init: dbus started"

ip link set lo up 2>/dev/null || true
NET_IFACE=""
for i in $(seq 1 40); do
    for iface_path in /sys/class/net/*; do
        iface="$(basename "$iface_path")"
        case "$iface" in
            lo|dummy*|sit*|ip6tnl*|tunl*) continue ;;
        esac
        NET_IFACE="$iface"
        break
    done
    [ -n "$NET_IFACE" ] && break
    sleep 0.25
done

if [ -n "$NET_IFACE" ]; then
    ip link set "$NET_IFACE" up 2>/dev/null || true
    dhcpcd -4 -w --timeout 15 "$NET_IFACE" >/tmp/dhcpcd.log 2>&1 &
    kmsg "fast-init: $NET_IFACE up, dhcpcd started"
else
    kmsg "fast-init: WARNING no virtio network interface found"
fi

kmsg "netdiag: ifaces=$(ip -o addr 2>&1 | tr '\n' '|')"
kmsg "netdiag: routes=$(ip route 2>&1 | tr '\n' '|')"
kmsg "netdiag: vsock=$(ls /dev/vsock 2>&1)"
kmsg "fast-init: complete"

while :; do
    kmsg "fast-init: launching compositor"
    /usr/local/bin/start-compositor
    status=$?
    kmsg "fast-init: compositor exited status=$status"
    sleep 1
done
FASTINIT
chmod 0755 /rootfs/sbin/fast-init
ln -sf fast-init /rootfs/sbin/init

echo "--- Building APC initramfs ---"
INITRAMFS_DIR="$(mktemp -d)"
mkdir -p "$INITRAMFS_DIR/bin" "$INITRAMFS_DIR/dev" "$INITRAMFS_DIR/newroot" "$INITRAMFS_DIR/proc" "$INITRAMFS_DIR/sys"
if [ -x /rootfs/bin/busybox ]; then
    cp /rootfs/bin/busybox "$INITRAMFS_DIR/bin/busybox"
elif [ -x /rootfs/usr/bin/busybox ]; then
    cp /rootfs/usr/bin/busybox "$INITRAMFS_DIR/bin/busybox"
else
    echo "ERROR: busybox-static did not install a busybox binary"
    exit 1
fi
cat > "$INITRAMFS_DIR/init" << 'INIT'
#!/bin/busybox sh
set -eu

export PATH=/bin
busybox mount -t proc proc /proc
busybox mount -t sysfs sysfs /sys
busybox mount -t devtmpfs devtmpfs /dev
busybox mkdir -p /newroot

for i in $(busybox seq 1 80); do
    [ -e /dev/vda ] && break
    busybox sleep 0.1
done

if [ ! -e /dev/vda ]; then
    echo "initramfs: ERROR /dev/vda not found" >/dev/kmsg
    exec /bin/busybox sh
fi

busybox mount -t ext4 -o rw /dev/vda /newroot
exec busybox switch_root /newroot /sbin/fast-init
INIT
chmod 0755 "$INITRAMFS_DIR/init" "$INITRAMFS_DIR/bin/busybox"
(
    cd "$INITRAMFS_DIR"
    find . -print0 | cpio --null -o -H newc 2>/dev/null | gzip -9
) > /output/initramfs
rm -rf "$INITRAMFS_DIR"
echo "    initramfs: $(du -h /output/initramfs | cut -f1)"

echo "--- Creating ext4 disk image (${DISK_SIZE_MB}MB) ---"
rm -f /output/disk.img
truncate -s "${DISK_SIZE_MB}M" /output/disk.img
mke2fs -q -F -t ext4 -L apc -d /rootfs /output/disk.img

echo "--- Debian rootfs build complete ---"
ls -lh /output/
BUILDSCRIPT

docker run --rm \
    --platform "$DOCKER_PLATFORM" \
    -v "$SCRIPT_DIR/rootfs:/overlay:ro" \
    -v "$OUT_DIR:/output" \
    -v "$BUILD_SCRIPT:/build.sh:ro" \
    "${BROWSERD_DOCKER_ARGS[@]}" \
    "$DEBIAN_IMAGE" \
    bash /build.sh "$DISK_SIZE_MB" "$DEBIAN_SUITE" "$DEBIAN_ARCH" "$DEBIAN_MIRROR" "$APC_BROWSERD"

rm -f "$BUILD_SCRIPT"

if [ "$ARCH" = "aarch64" ]; then
    KERNEL_IMAGE=$(find "$WORKSPACE_DIR/deps/src/libkrunfw" -path "*/arch/arm64/boot/Image" 2>/dev/null | head -1)
    if [ -n "$KERNEL_IMAGE" ]; then
        cp "$KERNEL_IMAGE" "$OUT_DIR/vmlinuz"
        echo "==> Kernel: copied from libkrunfw ($(du -h "$OUT_DIR/vmlinuz" | cut -f1))"
    elif [ -f "$OUT_DIR/vmlinuz" ]; then
        echo "==> Kernel: using existing $OUT_DIR/vmlinuz (libkrunfw source not found)"
    else
        echo "ERROR: No aarch64 kernel available. Run deps/build-deps.sh first to build libkrunfw."
        exit 1
    fi
elif [ -f "$OUT_DIR/vmlinuz" ]; then
    echo "==> Kernel: using existing $OUT_DIR/vmlinuz"
else
    echo "ERROR: No x86_64 kernel path is enabled until native x86_64 build and boot testing is done."
    exit 1
fi

echo ""
echo "==> Guest image built successfully"
echo "    Kernel:    $OUT_DIR/vmlinuz"
echo "    Initramfs: $OUT_DIR/initramfs"
echo "    Disk:      $OUT_DIR/disk.img"
echo ""
echo "    Run with:"
echo "    cargo run -p apc-host -- \\"
echo "      --kernel $OUT_DIR/vmlinuz \\"
echo "      --initrd $OUT_DIR/initramfs \\"
echo "      --disk $OUT_DIR/disk.img"
