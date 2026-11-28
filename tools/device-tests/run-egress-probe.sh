#!/bin/sh
# Run the egress gate without requiring Python in the guest.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
DEV=${DEV:?usage: DEV=<serial> sh run-egress-probe.sh}
PKG=${PKG:-com.linuxhub.myth.husk}
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
    echo "run-egress-probe: missing $RUNTIME_PROBE; run tools/guest-config/build-guest-tools.sh" >&2
    exit 1
}

cleanup() {
    adb -s "$DEV" shell "run-as $PKG rm -f '$ROOTFS/root/runtime-probe' '$ROOTFS/tmp/13-egress.sh'" >/dev/null 2>&1 || true
    adb -s "$DEV" shell 'rm -f /data/local/tmp/husk-runtime-probe /data/local/tmp/husk-13-egress.sh' >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$RUNTIME_PROBE" /data/local/tmp/husk-runtime-probe >/dev/null
adb -s "$DEV" push "$HERE/13-egress.sh" /data/local/tmp/husk-13-egress.sh >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'mkdir -p $ROOTFS/root $ROOTFS/tmp; cp /data/local/tmp/husk-runtime-probe $ROOTFS/root/runtime-probe; cp /data/local/tmp/husk-13-egress.sh $ROOTFS/tmp/13-egress.sh; chmod 755 $ROOTFS/root/runtime-probe $ROOTFS/tmp/13-egress.sh'"

adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/env RUNTIME_PROBE=/root/runtime-probe /usr/bin/bash /tmp/13-egress.sh"
