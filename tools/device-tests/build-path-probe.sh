#!/bin/sh
# Build the guest-side Rust path probe against an Ubuntu arm64 glibc sysroot.
# The probe must be run under the embedded loader, not executed on the host.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
# Where the NDK lives is not a property of the repository: resolve it from the
# environment (ANDROID_NDK_HOME, then ANDROID_HOME/ANDROID_SDK_ROOT) and only
# then guess the default SDK location for this host. A hard-coded home directory
# here used to leak one developer's layout into the tree.
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
CLANG=$NDK/bin/clang
LLD=$NDK/bin/ld.lld
SYSROOT=${GLIBC_SYSROOT:-/tmp/nr-toolchain/root}

[ -x "$CLANG" ] || { echo "missing clang: $CLANG" >&2; exit 1; }
[ -x "$LLD" ] || { echo "missing ld.lld: $LLD" >&2; exit 1; }
[ -d "$SYSROOT/usr/lib/aarch64-linux-gnu" ] || {
    echo "missing glibc sysroot: $SYSROOT (run provision-toolchain.sh or set GLIBC_SYSROOT)" >&2
    exit 1
}

rustc --target aarch64-unknown-linux-gnu \
    -C linker="$CLANG" \
    -C opt-level=2 \
    -C link-arg=--target=aarch64-linux-gnu \
    -C link-arg=--sysroot="$SYSROOT" \
    -C link-arg=-fuse-ld="$LLD" \
    -C link-arg=-L"$SYSROOT/usr/lib/aarch64-linux-gnu" \
    -C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1 \
    -o "$HERE/path-probe" "$HERE/path-probe.rs"

file "$HERE/path-probe"
