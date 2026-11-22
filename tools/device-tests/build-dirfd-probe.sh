#!/bin/sh
# Build the no_std dirfd containment probe against the guest's real libc.
#
# The probe must be dynamically linked to libc so its calls go through the
# loader-embedded shim's GOT interpose. This avoids depending on a full glibc-dev
# sysroot: only the runtime libc.so.6 from the same guest is needed.
#
# Example:
#   adb exec-out run-as com.linuxhub.husk \
#     cat files/rootfs-full/usr/lib/aarch64-linux-gnu/libc.so.6 > /tmp/husk-glibc.so.6
#   LIBC_SO=/tmp/husk-glibc.so.6 ./build-dirfd-probe.sh
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
NDK=${NDK:-/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64}
LIBC_SO=${LIBC_SO:?set LIBC_SO to the guest runtime libc.so.6}

[ -x "$NDK/bin/clang" ] || { echo "missing clang: $NDK/bin/clang" >&2; exit 1; }
[ -f "$LIBC_SO" ] || { echo "missing guest libc: $LIBC_SO" >&2; exit 1; }

rustc --target aarch64-unknown-linux-gnu \
    -C linker="$NDK/bin/clang" \
    -C link-arg=--target=aarch64-linux-gnu \
    -C link-arg=-nostdlib \
    -C link-arg=-nostartfiles \
    -C link-arg=-pie \
    -C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1 \
    -C link-arg=-Wl,-e,_start \
    -C link-arg=-Wl,--no-as-needed \
    -C link-arg="$LIBC_SO" \
    -C panic=abort \
    -C opt-level=2 \
    -o "$HERE/dirfd-probe" "$HERE/dirfd-probe.rs"

file "$HERE/dirfd-probe"
