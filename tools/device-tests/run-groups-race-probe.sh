#!/bin/sh
# Stress getgroups()/setgroups() while a signal handler re-enters getgroups().
set -eu
DEV=${DEV:-${1:-}}
[ -n "$DEV" ] || { echo "usage: $0 <device-serial>" >&2; exit 1; }
PKG=${PKG:-com.linuxhub.myth.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/distributions/ubuntu-24.04/rootfs}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
ROUNDS=${ROUNDS:-20}
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

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$HERE/../probe/groups-race-probe.c" /data/local/tmp/groups-race-probe.c >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/groups-race-probe.c $ROOTFS/root/groups-race-probe.c'"
if ! BUILD=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/gcc -O2 -pthread -o /root/groups-race /root/groups-race-probe.c" 2>&1); then
    echo '--- groups-race-probe: compile failed' >&2
    echo "$BUILD" >&2
    exit 1
fi
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/bash -c 'f=0; for i in \$(seq 1 $ROUNDS); do o=\$(/root/groups-race 2>&1) || { printf \"%s\\n\" \"\$o\"; f=\$((f+1)); }; done; echo GROUPS_RACE rounds=$ROUNDS fails=\$f'" 2>&1); then
    echo '--- groups-race-probe: execution failed' >&2
    echo "$OUT" >&2
    exit 1
fi
echo "$OUT"
echo "$OUT" | grep -qx "GROUPS_RACE rounds=$ROUNDS fails=0" || {
    echo '--- groups-race-probe: FAIL' >&2
    exit 1
}
adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/groups-race-probe.c $ROOTFS/root/groups-race" || true
adb -s "$DEV" shell 'rm -f /data/local/tmp/groups-race-probe.c' || true
echo '--- groups-race-probe: PASS'
