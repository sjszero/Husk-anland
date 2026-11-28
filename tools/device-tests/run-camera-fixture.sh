#!/bin/sh
# Build the camera fixture and exercise the guest V4L2 facade through the
# installed Husk app. This validates the Linux runtime without requiring a
# Camera2 permission or a real sensor.
set -eu

DEV=${1:?usage: $0 <adb-serial>}
PKG=${PKG:-com.linuxhub.myth.husk}
ROOTFS=${ROOTFS:-/data/user/0/$PKG/files/rootfs-full}
LD=${LD:-/data/user/0/$PKG/files/linux/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)

# Paths passed to adb shell are resolved by the app process, not by the caller's
# checkout. Normalize repository-relative `files/...` arguments before they
# cross the device boundary; otherwise the guest tries to load a host-relative
# path such as `files/husk-dist-current/libld.so` and cannot find it.
device_path() {
    case "$1" in
        files/*) printf '/data/data/%s/%s\n' "$PKG" "$1" ;;
        *) printf '%s\n' "$1" ;;
    esac
}
ROOTFS=$(device_path "$ROOTFS")
LD=$(device_path "$LD")
V4L2_PRELOAD=${V4L2_PRELOAD:-$ROOTFS/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so}
ROOT=$(cd "$HERE/../.." && pwd)

# The raw fixture temporarily replaces the app's guest-facing camera socket.
# Unlinking a listening Unix socket does not close the app's listener; it stays
# attached to the removed inode and cannot serve later gate runs. Restart the
# app on every exit path so the production bridge binds a fresh socket.
restart_app() {
    adb -s "$DEV" shell am force-stop "$PKG" >/dev/null 2>&1 || true
    launcher=$(adb -s "$DEV" shell "cmd package resolve-activity --brief $PKG" 2>/dev/null | tail -1 | tr -d '\r')
    if [ -n "$launcher" ]; then
        adb -s "$DEV" shell am start -W -n "$launcher" >/dev/null 2>&1 || true
    fi
    i=0
    while [ "$i" -lt 80 ]; do
        if adb -s "$DEV" shell "run-as $PKG test -S $ROOTFS/run/husk/camera.sock" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.25
        i=$((i + 1))
    done
    echo "camera-fixture: WARN app camera.sock did not return after restart" >&2
    return 1
}
trap 'restart_app >/dev/null 2>&1 || true' EXIT HUP INT TERM
NDK=${NDK:-${ANDROID_NDK_HOME:-${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64}}
SYSROOT=${SYSROOT:-/tmp/nr-toolchain/root}

case "$NDK" in */bin) NDK=${NDK%/bin} ;; esac
[ -x "$NDK/bin/clang" ] || { echo "missing NDK clang: $NDK/bin/clang" >&2; exit 69; }
[ -f "$SYSROOT/usr/lib/aarch64-linux-gnu/crt1.o" ] || {
    echo "missing glibc sysroot: $SYSROOT" >&2
    exit 69
}

cd "$ROOT"
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld \
    cargo build --manifest-path runtime/Cargo.toml \
    --target aarch64-unknown-linux-musl -p camera-fixture >/dev/null

FLAGS="-C linker=$NDK/bin/clang \
       -C link-arg=--target=aarch64-linux-gnu \
       -C link-arg=--sysroot=$SYSROOT \
       -C link-arg=-fuse-ld=$NDK/bin/ld.lld \
       -C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1 \
       -C link-arg=-Wl,-z,max-page-size=65536"
RUSTFLAGS="$FLAGS" cargo build --manifest-path runtime/Cargo.toml \
    --release --target aarch64-unknown-linux-gnu \
    -p camera-fixture -p v4l2-test >/dev/null

FIXTURE_HOST=$ROOT/runtime/target/aarch64-unknown-linux-musl/debug/camera-fixture
FIXTURE_GUEST=$ROOT/runtime/target/aarch64-unknown-linux-gnu/release/camera-fixture
V4L2_TEST=$ROOT/runtime/target/aarch64-unknown-linux-gnu/release/v4l2-test

for pair in \
    "$FIXTURE_HOST:/data/local/tmp/husk-camera-fixture-host" \
    "$FIXTURE_GUEST:/data/local/tmp/husk-camera-fixture" \
    "$V4L2_TEST:/data/local/tmp/husk-v4l2-test"; do
    src=${pair%:*}
    dst=${pair#*:}
    adb -s "$DEV" push "$src" "$dst" >/dev/null
done
adb -s "$DEV" push "$ROOT/tools/guest/husk-session.sh" "$SESS" >/dev/null

adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/husk-camera-fixture-host $ROOTFS/root/camera-fixture-host; cp /data/local/tmp/husk-camera-fixture $ROOTFS/root/camera-fixture; cp /data/local/tmp/husk-v4l2-test $ROOTFS/root/v4l2-test; chmod 755 $ROOTFS/root/camera-fixture-host $ROOTFS/root/camera-fixture $ROOTFS/root/v4l2-test'"

run_raw() {
    name=$1
    width=$2
    height=$3
    adb -s "$DEV" shell "run-as $PKG sh -c '
        rm -f $ROOTFS/run/husk/camera.sock
        $ROOTFS/root/camera-fixture-host $ROOTFS/run/husk/camera.sock 10 $width $height 1 >$ROOTFS/root/fixture-$name.log 2>&1 &
        p=\$!
        sleep 1
        NR_LD=$LD sh $SESS $ROOTFS -- /root/v4l2-test /dev/video0 10 NV12 /tmp/frame-$name $width $height
        rc=\$?
        if [ \$rc -ne 0 ]; then
            kill \$p 2>/dev/null || true
        fi
        wait \$p || true
        cat $ROOTFS/root/fixture-$name.log
        exit \$rc
    '"
}

run_gst() {
    name=$1
    width=$2
    height=$3
    out=$(adb -s "$DEV" shell "run-as $PKG sh -c '
        rm -f $ROOTFS/run/husk/camera.sock $ROOTFS/root/v4l2-audit-$name.log
        NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/env \
            LD_PRELOAD=$V4L2_PRELOAD \
            NR_V4L2_AUDIT=1 \
            NR_SIGSYS_LOG=$ROOTFS/root/v4l2-audit-$name.log \
            GST_PLUGIN_SYSTEM_PATH_1_0=/usr/lib/aarch64-linux-gnu/gstreamer-1.0 \
            GST_REGISTRY=/tmp/husk-gst-registry.bin \
            GST_REGISTRY_UPDATE=yes GST_DEBUG=2 \
            /root/camera-fixture --gst $width $height 3
        rc=\$?
        echo \"--- v4l2 audit $name ---\"
        cat $ROOTFS/root/v4l2-audit-$name.log 2>/dev/null || true
        exit \$rc
    '" 2>&1) || { printf '%s\n' "$out"; exit 1; }
    printf '%s\n' "$out"
    if printf '%s\n' "$out" | grep -Eq \
        'VIDIOC_CROPCAP failed|Timestamp does not correlate with any clock'; then
        echo "camera-fixture: FAIL (V4L2 crop/timestamp warning regressed)" >&2
        exit 1
    fi
}

run_raw 640 640 480
run_raw 720 1280 720
run_gst 640 640 480
run_gst 720 1280 720

echo "camera-fixture: PASS (raw + GStreamer, 640x480 and 1280x720)"
