#!/bin/sh
# Run the guest-side V4L2 fd-lifetime regression probe through Husk itself.
set -eu
DEV=${1:?usage: $0 <device-serial>}
PKG=${PKG:-com.linuxhub.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/../rootfs-full}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)
case "$ROOTFS" in
    files/linux/*) ROOTFS=$APPDIR/${ROOTFS#files/linux/} ;;
    files/rootfs-full|files/rootfs) ROOTFS=/data/data/$PKG/$ROOTFS ;;
esac
case "$LD" in
    files/linux/*) LD=$APPDIR/${LD#files/linux/} ;;
esac

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld \
    cargo build --manifest-path runtime/Cargo.toml \
    --target aarch64-unknown-linux-musl -p v4l2-ref-probe >/dev/null
BIN=runtime/target/aarch64-unknown-linux-musl/debug/v4l2-ref-probe
adb -s "$DEV" push "$BIN" /data/local/tmp/v4l2-ref-probe >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/v4l2-ref-probe $ROOTFS/root/v4l2-ref-probe && chmod 755 $ROOTFS/root/v4l2-ref-probe'"
adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/v4l2-ref-probe"
adb -s "$DEV" shell "run-as $PKG sh -c 'rm -f $ROOTFS/root/v4l2-ref-probe'"
adb -s "$DEV" shell 'rm -f /data/local/tmp/v4l2-ref-probe'
