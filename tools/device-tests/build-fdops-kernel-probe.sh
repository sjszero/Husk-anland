#!/bin/sh
# Build the static no_std raw-syscall metadata probe.
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
