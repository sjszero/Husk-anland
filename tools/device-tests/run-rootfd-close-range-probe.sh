#!/bin/sh
# Verify that concurrent close_range(rootfd, rootfd) cannot close the private
# root descriptor while other threads resolve guest paths.
set -eu
DEV=${DEV:-${1:-}}
[ -n "$DEV" ] || { echo "usage: $0 <device-serial>" >&2; exit 1; }
PKG=${PKG:-com.linuxhub.myth.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/../rootfs-full}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
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
    echo "--- rootfd-close-range-probe: SKIP (missing $RUNTIME_PROBE; run tools/guest-config/build-guest-tools.sh)" >&2
    exit 0
}
adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$RUNTIME_PROBE" /data/local/tmp/husk-runtime-probe >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/husk-runtime-probe $ROOTFS/root/runtime-probe && chmod 755 $ROOTFS/root/runtime-probe'"
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/runtime-probe rootfd-close-range" 2>&1); then
    echo '--- rootfd-close-range-probe: execution failed' >&2
    echo "$OUT" >&2
    exit 1
fi
echo "$OUT"
echo "$OUT" | grep -q '^ROOTFD_RACE stat_ok=[0-9][0-9]* stat_fail=0 access_ok=[0-9][0-9]* access_fail=0 range_ok=[0-9][0-9]* range_fail=0 first_errno=0$' || {
    echo '--- rootfd-close-range-probe: FAIL' >&2
    exit 1
}
adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/runtime-probe" || true
adb -s "$DEV" shell 'rm -f /data/local/tmp/husk-runtime-probe' || true
echo '--- rootfd-close-range-probe: PASS'
