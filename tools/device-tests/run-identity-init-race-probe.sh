#!/bin/sh
# Fresh-process stress for the first fake getuid()/setuid() interleaving.
# The race is proved by construction in path-shim; this checks the final
# process identity over repeated fresh executions and catches regressions.
set -eu
DEV=${DEV:-${1:-}}
[ -n "$DEV" ] || { echo "usage: $0 <device-serial>" >&2; exit 1; }
PKG=${PKG:-com.linuxhub.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/distributions/ubuntu-24.04/rootfs}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
ROUNDS=${ROUNDS:-300}
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
    echo '--- identity-init-race-probe: SKIP  (rootfs 里没有 /usr/bin/gcc；这不是运行时缺陷)'
    exit 0
fi

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$HERE/../probe/identity-init-race-probe.c" /data/local/tmp/identity-init-race-probe.c >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/identity-init-race-probe.c $ROOTFS/root/identity-init-race-probe.c'"
if ! BUILD=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/gcc -O2 -pthread -o /root/identity-init-race /root/identity-init-race-probe.c" 2>&1); then
    echo '--- identity-init-race-probe: compile failed' >&2
    echo "$BUILD" >&2
    exit 1
fi
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/bash -c 'f=0; for i in \$(seq 1 $ROUNDS); do /root/identity-init-race >/dev/null || f=\$((f+1)); done; echo IDENTITY_RACE rounds=$ROUNDS fails=\$f'" 2>&1); then
    echo '--- identity-init-race-probe: execution failed' >&2
    echo "$OUT" >&2
    exit 1
fi
echo "$OUT"
echo "$OUT" | grep -qx "IDENTITY_RACE rounds=$ROUNDS fails=0" || {
    echo '--- identity-init-race-probe: FAIL' >&2
    exit 1
}
adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/identity-init-race-probe.c $ROOTFS/root/identity-init-race" || true
adb -s "$DEV" shell 'rm -f /data/local/tmp/identity-init-race-probe.c' || true
echo '--- identity-init-race-probe: PASS'
