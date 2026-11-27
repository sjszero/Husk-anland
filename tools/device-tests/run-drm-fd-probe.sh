#!/bin/sh
# Regression for fake-DRM descriptor identity across dup2/dup3 overwrites.
#
# A destination descriptor is closed before the source is installed. If the
# shim keeps the destination's old DRM table entry, ioctls on the replacement
# file wrongly enter DRM_CTX. The probe checks both directions:
#   ordinary -> fake destination: must leave the DRM shim and fail normally;
#   fake -> new descriptor:       must remain DRM after the source is closed.
set -eu
DEV=${1:?usage: $0 <device-serial>}
PKG=${PKG:-com.linuxhub.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/../rootfs-full}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

device_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        files/linux/*) printf '%s/%s\n' "$APPDIR" "${1#files/linux/}" ;;
        files/rootfs-full/*) printf "/data/data/%s/%s\n" "$PKG" "$1" ;;
        files/rootfs/*) printf "/data/data/%s/%s\n" "$PKG" "$1" ;;
        files/rootfs-full|files/rootfs) printf '/data/data/%s/%s\n' "$PKG" "$1" ;;
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

rustc --edition=2021 --target aarch64-unknown-linux-gnu \
    -C linker="$NDK_BIN/clang" \
    -C opt-level=2 \
    -C link-arg=--target=aarch64-linux-gnu \
    -C link-arg=--sysroot="$SYSROOT" \
    -C link-arg=-fuse-ld="$NDK_BIN/ld.lld" \
    -C link-arg=-L"$SYSROOT/usr/lib/aarch64-linux-gnu" \
    -C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1 \
    -o "$TMP/drmfake" "$HERE/../../runtime/crates/drm-probe/src/bin/drmfake.rs"

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$TMP/drmfake" /data/local/tmp/drmfake >/dev/null
if ! adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/drmfake $ROOTFS/root/drmfake && chmod 755 $ROOTFS/root/drmfake'"; then
    echo '--- drm-fd-probe: install failed' >&2
    exit 1
fi
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/drmfake" 2>&1); then
    echo '--- drm-fd-probe: execution failed' >&2
    echo "$OUT" >&2
    exit 1
fi
echo "$OUT"

fail=0
want() {
    if echo "$OUT" | grep -Fq "$1"; then
        printf 'ok   %s\n' "$2"
    else
        printf 'FAIL %s   (expected: %s)\n' "$2" "$1"
        fail=1
    fi
}

want 'drmfake|verdict|PASS|' '基础 DRM interpose 仍通过'
want 'drmfake|dup2-ordinary-over-fake|PASS|' 'dup2 普通文件覆盖假 fd 后不再进入 DRM'
want 'drmfake|dup3-ordinary-over-fake|PASS|' 'dup3 普通文件覆盖假 fd 后不再进入 DRM'
want 'drmfake|dup2-fake-preserved|PASS|' '假 DRM fd 的 dup2 副本在源 fd 关闭后仍有效'

adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/drmfake" || true
adb -s "$DEV" shell 'rm -f /data/local/tmp/drmfake' || true

if [ "$fail" -ne 0 ]; then
    echo '--- drm-fd-probe: FAIL' >&2
    exit 1
fi
echo '--- drm-fd-probe: PASS'
