#!/bin/sh
# Run the complete server-surface parity matrix without guest Python or awk.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
DEV=${DEV:?usage: DEV=<serial> sh run-server-parity.sh}
PKG=${PKG:-com.linuxhub.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
RUNTIME_PROBE=${RUNTIME_PROBE:-$REPO_ROOT/runtime/target/aarch64-unknown-linux-gnu/release/runtime-probe}

device_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        files/linux/*) printf '%s/%s\n' "$APPDIR" "${1#files/linux/}" ;;
        files/*) printf '/data/data/%s/%s\n' "$PKG" "$1" ;;
        *) printf '%s\n' "$1" ;;
    esac
}

ROOTFS=$(device_path "$ROOTFS")
LD=$(device_path "$LD")

[ -x "$RUNTIME_PROBE" ] || {
    echo "run-server-parity: missing $RUNTIME_PROBE; run tools/guest-config/build-guest-tools.sh" >&2
    exit 1
}

cleanup() {
    adb -s "$DEV" shell "run-as $PKG rm -f '$ROOTFS/root/runtime-probe' '$ROOTFS/tmp/rt-server-parity.sh'" >/dev/null 2>&1 || true
    adb -s "$DEV" shell 'rm -f /data/local/tmp/husk-runtime-probe /data/local/tmp/husk-rt-server-parity.sh' >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$RUNTIME_PROBE" /data/local/tmp/husk-runtime-probe >/dev/null
adb -s "$DEV" push "$HERE/rt-server-parity.sh" /data/local/tmp/husk-rt-server-parity.sh >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'mkdir -p $ROOTFS/root $ROOTFS/tmp; cp /data/local/tmp/husk-runtime-probe $ROOTFS/root/runtime-probe; cp /data/local/tmp/husk-rt-server-parity.sh $ROOTFS/tmp/rt-server-parity.sh; chmod 755 $ROOTFS/root/runtime-probe $ROOTFS/tmp/rt-server-parity.sh'"

adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/env RUNTIME_PROBE=/root/runtime-probe /usr/bin/bash /tmp/rt-server-parity.sh"
