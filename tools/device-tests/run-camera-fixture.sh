#!/bin/sh
# Build the camera fixture and exercise the guest V4L2 facade through the
# installed Husk app. This validates the Linux runtime without requiring a
# Camera2 permission or a real sensor.
set -eu

DEV=${1:?usage: $0 <adb-serial>}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-/data/user/0/$PKG/files/linux/distributions/ubuntu-24.04/rootfs}
LD=${LD:-/data/user/0/$PKG/files/linux/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
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
