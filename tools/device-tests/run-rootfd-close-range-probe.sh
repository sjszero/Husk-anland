#!/bin/sh
# Verify that concurrent close_range(rootfd, rootfd) cannot close the private
# root descriptor while other threads resolve guest paths.
set -eu
DEV=${DEV:-${1:-}}
[ -n "$DEV" ] || { echo "usage: $0 <device-serial>" >&2; exit 1; }
PKG=${PKG:-com.linuxhub.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/distributions/ubuntu-24.04/rootfs}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)

device_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        files/linux/*) printf '%s/%s\n' "$APPDIR" "${1#files/linux/}" ;;
        *) printf '%s\n' "$1" ;;
    esac
}
ROOTFS=$(device_path "$ROOTFS")
LD=$(device_path "$LD")

# This is a runtime semantic probe, not a requirement that every distro tree
# bundles a compiler. Keep the gate honest and actionable when gcc is absent.
if ! adb -s "$DEV" shell "run-as $PKG test -x $ROOTFS/usr/bin/gcc" >/dev/null 2>&1; then
    echo '--- rootfd-close-range-probe: SKIP  (rootfs 里没有 /usr/bin/gcc；这不是运行时缺陷)'
    exit 0
fi

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$HERE/../probe/rootfd-close-range-probe.c" /data/local/tmp/rootfd-close-range-probe.c >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/rootfd-close-range-probe.c $ROOTFS/root/rootfd-close-range-probe.c'"
if ! BUILD=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/gcc -O2 -pthread -o /root/rootfd-close-range /root/rootfd-close-range-probe.c" 2>&1); then
    echo '--- rootfd-close-range-probe: compile failed' >&2
    echo "$BUILD" >&2
    exit 1
fi
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/rootfd-close-range" 2>&1); then
    echo '--- rootfd-close-range-probe: execution failed' >&2
    echo "$OUT" >&2
    exit 1
fi
echo "$OUT"
echo "$OUT" | grep -q '^ROOTFD_RACE .*stat_fail=0 .*access_fail=0 .*range_fail=0 .*first_errno=0$' || {
    echo '--- rootfd-close-range-probe: FAIL' >&2
    exit 1
}
adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/rootfd-close-range-probe.c $ROOTFS/root/rootfd-close-range" || true
adb -s "$DEV" shell 'rm -f /data/local/tmp/rootfd-close-range-probe.c' || true
echo '--- rootfd-close-range-probe: PASS'
