#!/bin/sh
# Verify that dpkg can reopen a restrictive conffile under fake root without
# changing the package's on-disk mode.
#
# The guest half builds a throwaway package whose conffile is 0440. dpkg applies
# that mode before its final write/open; the shim must make the reopen possible
# and restore the inode mode before returning the writable fd.
set -eu

if [ "${1:-}" = "--guest" ]; then
    P=husk-conffile-probe
    W=/tmp/$P-build
    cleanup() {
        dpkg --purge "$P" >/dev/null 2>&1 || true
        rm -rf "$W" /etc/"$P"
    }
    trap cleanup 0 1 2 15

    rm -rf "$W"
    mkdir -p "$W/pkg/DEBIAN" "$W/pkg/etc"
    cat >"$W/pkg/DEBIAN/control" <<CTL
Package: $P
Version: 1.0
Section: misc
Priority: optional
Architecture: all
Maintainer: Husk <husk@example.invalid>
Description: Husk conffile mode probe
CTL
    printf '/etc/%s\n' "$P" >"$W/pkg/DEBIAN/conffiles"
    printf 'conffile\n' >"$W/pkg/etc/$P"
    chmod 0440 "$W/pkg/etc/$P"
    dpkg-deb --build "$W/pkg" "$W/$P.deb" >/dev/null
    dpkg --purge "$P" >/dev/null 2>&1 || true
    rm -f /etc/"$P"
    dpkg -i "$W/$P.deb" >/dev/null

    mode=$(stat -c '%a' /etc/"$P")
    printf 'DPKG_CONFFILE_MODE|%s|mode=%s expected=440\n' \
        "$([ "$mode" = 440 ] && echo PASS || echo FAIL)" "$mode"
    [ "$mode" = 440 ]
    exit 0
fi

DEV=${1:?usage: $0 <device-serial>}
PKG=${PKG:-com.linuxhub.myth.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/../rootfs-full}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)

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

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$HERE/run-dpkg-conffile-mode-probe.sh" /data/local/tmp/husk-dpkg-mode-probe.sh >/dev/null
if ! adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/husk-dpkg-mode-probe.sh $ROOTFS/root/husk-dpkg-mode-probe.sh && chmod 755 $ROOTFS/root/husk-dpkg-mode-probe.sh'"; then
    echo '--- dpkg-conffile-mode-probe: install failed' >&2
    exit 1
fi
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/husk-dpkg-mode-probe.sh --guest" 2>&1); then
    echo '--- dpkg-conffile-mode-probe: execution failed' >&2
    echo "$OUT" >&2
    adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/husk-dpkg-mode-probe.sh" || true
    adb -s "$DEV" shell 'rm -f /data/local/tmp/husk-dpkg-mode-probe.sh' || true
    exit 1
fi
echo "$OUT"

adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/husk-dpkg-mode-probe.sh" || true
adb -s "$DEV" shell 'rm -f /data/local/tmp/husk-dpkg-mode-probe.sh' || true

if ! echo "$OUT" | grep -Fq 'DPKG_CONFFILE_MODE|PASS|mode=440 expected=440'; then
    echo '--- dpkg-conffile-mode-probe: FAIL' >&2
    exit 1
fi
echo '--- dpkg-conffile-mode-probe: PASS'
