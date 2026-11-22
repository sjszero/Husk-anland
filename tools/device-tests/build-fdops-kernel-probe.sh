#!/bin/sh
# Build the static no_std raw-syscall metadata probe.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
NDK=${NDK:-/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64}

[ -x "$NDK/bin/clang" ] || { echo "missing clang: $NDK/bin/clang" >&2; exit 1; }

rustc --target aarch64-unknown-linux-gnu \
    -C linker="$NDK/bin/clang" \
    -C link-arg=--target=aarch64-linux-gnu \
    -C link-arg=-static \
    -C link-arg=-nostdlib \
    -C panic=abort \
    -C opt-level=2 \
    -o "$HERE/fdops-kernel-probe" "$HERE/fdops-kernel-probe.rs"

file "$HERE/fdops-kernel-probe"
