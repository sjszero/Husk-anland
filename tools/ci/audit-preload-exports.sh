#!/bin/sh
# Assert the two LD_PRELOAD layers still export what their clients bind to.
#
# Why this exists: both layers are `#[no_mangle]` cdylibs, and both are loaded
# by dlopen at runtime (Mesa's libgbm/libdrm, GStreamer's libgstvideo4linux2).
# Nothing in the tree links against them, so if a symbol stops being exported -
# an item inserted between `#[no_mangle]` and its `fn`, a rename, a cfg - every
# `cargo check`, `clippy` and unit test still passes and the layer silently
# stops intercepting. The v4l2 layer additionally binds *versioned* names
# (open64@GLIBC_2.17 ...), which only exist if exports.map is on the link line;
# without it the release build does not link at all, which is the other half of
# this check.
#
# Usage: audit-preload-exports.sh [<llvm-nm>]
set -eu
cd "$(dirname "$0")/../.."

NM=${1:-${NM:-}}
if [ -z "$NM" ]; then
    for cand in llvm-nm \
                "${ANDROID_NDK_LATEST_HOME:-/nonexistent}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-nm" \
                "${NDK:-/nonexistent}/llvm-nm"; do
        if command -v "$cand" >/dev/null 2>&1; then NM=$cand; break; fi
    done
fi
[ -n "$NM" ] || { echo "audit-preload-exports: no llvm-nm (pass as \$1 or set NM)" >&2; exit 1; }

D=runtime/target/aarch64-unknown-linux-gnu/release
DRM=$D/libdrm_preload.so
V4L2=$D/libhusk_v4l2.so

for so in "$DRM" "$V4L2"; do
    [ -f "$so" ] || { echo "audit-preload-exports: $so missing (run make preloads)" >&2; exit 1; }
done

fail=0
exported() { "$NM" -D --defined-only "$1" | awk '{print $3}'; }

need() { # need <so> <symbol>
    if exported "$1" | grep -qx "$2"; then
        echo "OK   $(basename "$1") $2"
    else
        echo "FAIL $(basename "$1") does not export $2" >&2
        fail=1
    fi
}

# drm layer: the fake node is a real /dev/mali0 fd, so it does not interpose
# close(); it exposes the node open/ioctl/mmap entry points plus the identity
# probe the path shim and fd-identity-probe dlsym.
for s in open open64 openat ioctl mmap husk_drm_is_fake husk_drm_ioctl \
         husk_drm_fake_open husk_drm_fake_open_flags drm_shim_is_fake; do
    need "$DRM" "$s"
done

# v4l2 layer: the versioned names are the whole point - an unversioned
# definition does not satisfy libgstvideo4linux2's versioned references.
for s in open64@GLIBC_2.17 open@GLIBC_2.17 openat@GLIBC_2.17 \
         ioctl@GLIBC_2.17 mmap64@GLIBC_2.17 close@GLIBC_2.17 \
         read@GLIBC_2.17 readv@GLIBC_2.17 stat64@GLIBC_2.33; do
    need "$V4L2" "$s"
done
# `husk_v4l2_read` does not exist: read() is served through the versioned
# read/readv interposers above and through `v4l2_read`, so assert the names that
# are actually there rather than the names that sound right.
for s in husk_v4l2_open husk_v4l2_ioctl husk_v4l2_is_fake husk_v4l2_mmap \
         husk_v4l2_close husk_v4l2_nonblock v4l2_read; do
    need "$V4L2" "$s"
done

[ "$fail" -eq 0 ] && echo "preload exports OK"
exit "$fail"
