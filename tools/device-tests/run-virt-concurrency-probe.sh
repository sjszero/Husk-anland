#!/bin/sh
# Rebuild and run the synthetic-proc concurrency probe on a real device.
#
# usage: sh tools/device-tests/run-virt-concurrency-probe.sh <serial> <package> <rootfs> <loader> [rounds]
set -eu
DEV=${1:?device serial}
PKG=${2:?package}
ROOTFS=${3:?rootfs device path or files/... path}
LD=${4:?loader device path or files/... path}
ROUNDS=${5:-4000}
HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

APPDIR=/data/user/0/$PKG/files
device_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        files/*) printf '%s/%s\n' "$APPDIR" "${1#files/}" ;;
        *) printf '%s\n' "$1" ;;
    esac
}
ROOTFS=$(device_path "$ROOTFS")
LD=$(device_path "$LD")

if [ -z "${NDK:-}" ]; then
    if [ -n "${ANDROID_NDK_HOME:-}" ]; then
        NDK=$ANDROID_NDK_HOME
    else
        case "$(uname -s)" in
            Darwin) ndk_host=darwin-x86_64 ;;
            *)      ndk_host=linux-x86_64 ;;
        esac
        NDK=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}/ndk/29.0.14206865/toolchains/llvm/prebuilt/$ndk_host
    fi
fi
case "$NDK" in
    */bin) NDK_BIN=$NDK ;;
    *) NDK_BIN=$NDK/bin ;;
esac
SYSROOT=${GLIBC_SYSROOT:-/tmp/nr-toolchain/root}
[ -x "$NDK_BIN/clang" ] || { echo "missing clang: $NDK_BIN/clang" >&2; exit 1; }
[ -d "$SYSROOT/usr/lib/aarch64-linux-gnu" ] || {
    echo "missing glibc sysroot: $SYSROOT" >&2
    exit 1
}

rustc --target aarch64-unknown-linux-gnu \
    -C linker="$NDK_BIN/clang" \
    -C opt-level=2 \
    -C link-arg=--target=aarch64-linux-gnu \
    -C link-arg=--sysroot="$SYSROOT" \
    -C link-arg=-fuse-ld="$NDK_BIN/ld.lld" \
    -C link-arg=-L"$SYSROOT/usr/lib/aarch64-linux-gnu" \
    -C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1 \
    -o "$TMP/virt-concurrency-probe" "$HERE/virt-concurrency-probe.rs"

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" /data/local/tmp/husk-session.sh >/dev/null
adb -s "$DEV" push "$TMP/virt-concurrency-probe" /data/local/tmp/virt-concurrency-probe >/dev/null
adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/virt-concurrency-probe $ROOTFS/root/virt-concurrency-probe"
adb -s "$DEV" shell "run-as $PKG chmod 755 $ROOTFS/root/virt-concurrency-probe"
adb -s "$DEV" shell "run-as $PKG env NR_LD='$LD' sh /data/local/tmp/husk-session.sh '$ROOTFS' -- /root/virt-concurrency-probe '$ROUNDS'"
