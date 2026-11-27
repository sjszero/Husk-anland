#!/bin/sh
# Stage the Rust supplementary-group probe, run the guest fixture under the
# Husk loader, and remove only the files this gate created.
set -eu
DEV=${DEV:-${1:-}}
[ -n "$DEV" ] || { echo "usage: $0 <device-serial>" >&2; exit 1; }
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
RUNTIME_PROBE=${RUNTIME_PROBE:-$REPO_ROOT/runtime/target/aarch64-unknown-linux-gnu/release/runtime-probe}
REMOTE=/data/local/tmp/husk-runtime-probe-$$

[ -x "$RUNTIME_PROBE" ] || {
    echo "--- supplementary-groups-probe: SKIP (missing $RUNTIME_PROBE; run tools/guest-config/build-guest-tools.sh)" >&2
    exit 0
}

cleanup() {
    adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/runtime-probe $ROOTFS/audit-supplementary-groups.sh" >/dev/null 2>&1 || true
    adb -s "$DEV" shell "rm -f $REMOTE" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

adb -s "$DEV" push "$RUNTIME_PROBE" "$REMOTE" >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'mkdir -p $ROOTFS/root && cp $REMOTE $ROOTFS/root/runtime-probe && chmod 755 $ROOTFS/root/runtime-probe'"
DEV="$DEV" PKG="$PKG" ROOTFS="$ROOTFS" LD="$LD" \
    ENV=RUNTIME_PROBE=/root/runtime-probe \
    sh "$HERE/run-guest.sh" audit-supplementary-groups.sh
