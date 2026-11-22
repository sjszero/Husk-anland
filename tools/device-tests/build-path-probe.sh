#!/bin/sh
# Build the guest-side Rust path probe against an Ubuntu arm64 glibc sysroot.
# The probe must be run under the embedded loader, not executed on the host.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
NDK=${NDK:-/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64}
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
