#!/bin/sh
# Dynamic identity regression for synthetic V4L2, netlink and DRM descriptors.
#
# Runs a glibc Rust probe through the Husk loader and exercises both directions:
#   ordinary -> fake destination: the destination must leave the emulation;
#   fake -> copied descriptor:    the copy must keep the fake identity.
#
# The DRM half dlopens the preload directly and exercises its exported identity check.
#
# The V4L2 half uses an in-process camera.sock protocol impostor; no real camera
# or app-side permission is required.
set -eu
DEV=${1:?usage: $0 <device-serial>}
PKG=${PKG:-com.linuxhub.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/../rootfs-full}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)

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

# The probe must exercise the loader that was just built. Three copies of
# libld.so exist on the device (the APK's native lib dir, files/linux/, and
# files/linux/husk/) and this script's default $LD is the third one. Writing a
# fresh loader into the second copy made a fixed bug look unfixed for hours
# while every run was really still executing the 19:04 build (evidence 294 §1).
# Refuse to run at all when the bytes differ; a probe result is only worth
# something when you know which loader produced it.
local_sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
LOCAL_LD="$(cd "$HERE/../.." && pwd)/tools/loader/ld-embedded.so"
if [ -f "$LOCAL_LD" ]; then
    want_sha=$(local_sha "$LOCAL_LD")
    got_sha=$(adb -s "$DEV" shell "run-as $PKG sha256sum $LD" 2>/dev/null | awk '{print $1}' | tr -d '\r')
    if [ "$want_sha" != "$got_sha" ]; then
        echo "--- loader mismatch: refusing to run" >&2
        echo "    device $LD" >&2
        echo "      sha256=$got_sha" >&2
        echo "    built  $LOCAL_LD" >&2
        echo "      sha256=$want_sha" >&2
        exit 1
    fi
    echo "# loader under test: $LD sha256=$got_sha (matches local build)"
fi

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

FLAGS="-C linker=$NDK_BIN/clang \
       -C link-arg=--target=aarch64-linux-gnu \
       -C link-arg=--sysroot=$SYSROOT \
       -C link-arg=-fuse-ld=$NDK_BIN/ld.lld \
       -C link-arg=-L$SYSROOT/usr/lib/aarch64-linux-gnu \
       -C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1 \
       -C link-arg=-Wl,-z,max-page-size=65536"
RUSTFLAGS="$FLAGS" cargo build --manifest-path runtime/Cargo.toml \
    --release --target aarch64-unknown-linux-gnu -p fd-identity-probe -p drm-preload >/dev/null
BIN=runtime/target/aarch64-unknown-linux-gnu/release/fd-identity-probe
PRELOAD=runtime/target/aarch64-unknown-linux-gnu/release/libdrm_preload.so

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$BIN" /data/local/tmp/fd-identity-probe >/dev/null
adb -s "$DEV" push "$PRELOAD" /data/local/tmp/libhusk_drm.so >/dev/null
if ! adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/fd-identity-probe $ROOTFS/root/fd-identity-probe && chmod 755 $ROOTFS/root/fd-identity-probe && cp /data/local/tmp/libhusk_drm.so $ROOTFS/root/libhusk_drm.so && chmod 755 $ROOTFS/root/libhusk_drm.so'"; then
    echo '--- fd-identity-probe: install failed' >&2
    exit 1
fi
if ! OUT=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /root/fd-identity-probe" 2>&1); then
    echo '--- fd-identity-probe: execution failed' >&2
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

want 'fdidentity|pathshim-fchown-owner|PASS|' 'path-shim 的 fchown 跨进程保留虚构所有者'
want 'fdidentity|pathshim-v4l2-open-flags|PASS|' 'path-shim 的 V4L2 open 保留 O_CLOEXEC/O_NONBLOCK'
want 'fdidentity|pathshim-netlink-socket-flags|PASS|' 'path-shim 的 netlink socket 保留 SOCK_CLOEXEC/SOCK_NONBLOCK'
want 'fdidentity|v4l2-dup2-ordinary-over-fake|PASS|' 'V4L2 dup2 普通文件覆盖假 fd 后离开模拟层'
want 'fdidentity|v4l2-dup3-ordinary-over-fake|PASS|' 'V4L2 dup3 普通文件覆盖假 fd 后离开模拟层'
want 'fdidentity|v4l2-dup2-fake-preserved|PASS|' 'V4L2 假 fd 的 dup2 副本在源关闭后仍有效'
want 'fdidentity|netlink-dup2-ordinary-over-fake|PASS|' 'netlink dup2 普通 socket 覆盖假 fd 后离开模拟层'
want 'fdidentity|netlink-dup3-ordinary-over-fake|PASS|' 'netlink dup3 普通 socket 覆盖假 fd 后离开模拟层'
want 'fdidentity|netlink-dup2-fake-preserved|PASS|' 'netlink 假 fd 的 dup2 副本在源关闭后仍是 AF_NETLINK'
want 'fdidentity|drm-preload-dup2-ordinary-over-fake|PASS|' 'drm preload 的假 fd 被普通文件覆盖后不再认成 DRM'
want 'fdidentity|drm-preload-dup2-fake-preserved|PASS|' 'drm preload 的假 fd 复制副本在源关闭后仍有效'
want 'fdidentity|pathshim-drm-open-cloexec|PASS|' 'path-shim 的 DRM open 保留 O_CLOEXEC'
want 'fdidentity|drm-preload-open-failure-abi|PASS|' 'drm preload 的 open 失败保持 -1/errno ABI'
want 'fdidentity|drm-preload-openat-failure-errno|PASS|' 'drm preload 的 openat 失败不把 ENOENT 改写成 EPERM'
want 'fdidentity|drm-preload-openat-create-mode|PASS|' 'drm preload 的 openat 创建文件保留请求 mode'
want 'fdidentity|drm-preload-open-cloexec|PASS|' 'drm preload 的 DRM open 保留 O_CLOEXEC'
want 'fdidentity|drm-preload-mmap-failure-errno|PASS|' 'drm preload 的 mmap 失败返回 MAP_FAILED 并保留 errno'
want 'fdidentity|drm-preload-ioctl-failure-errno|PASS|' 'drm preload 的普通 fd ioctl 失败保留 ENOTTY'
# 第 3 轮的两条：没有状态的假 DRM fd 必须 EOPNOTSUPP（不是 EBADF、不是伪成功），
# 假 V4L2 的名字必须整串匹配（越界下标/尾巴不对/拿了 DRM 的名字都不认领）。
want 'fdidentity|pathshim-drm-fake-fd-identity-lost|PASS|' '无状态假 DRM fd 明确回 EOPNOTSUPP'
want 'fdidentity|v4l2-name-grammar-exact|PASS|' '假 V4L2 fd 名字整串匹配，越界与脏尾巴不认领'
want 'fdidentity|pathshim-drm-fake-fd-identity-lost-preload|PASS|' 'preload 在场时同一条件统一回 EOPNOTSUPP(95)'

adb -s "$DEV" shell "run-as $PKG rm -f $ROOTFS/root/fd-identity-probe $ROOTFS/root/libhusk_drm.so" || true
adb -s "$DEV" shell 'rm -f /data/local/tmp/fd-identity-probe /data/local/tmp/libhusk_drm.so' || true

if [ "$fail" -ne 0 ]; then
    echo '--- fd-identity-probe: FAIL' >&2
    exit 1
fi
echo '--- fd-identity-probe: PASS'
