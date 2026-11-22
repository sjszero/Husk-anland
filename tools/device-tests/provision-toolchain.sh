#!/bin/sh
# Provision a C toolchain (gcc 13, binutils, make, a POSIX shell) inside the
# guest rootfs that lives in the app's own private storage.
#
#   ./provision-toolchain.sh              # resolve, download, unpack, push
#   NO_PUSH=1 ./provision-toolchain.sh    # stop before pushing
#
# The implementation is `nr-tools provision` (Rust, runtime/crates/nr-tools), the
# same crate as the loader surgery. The sh+python version this port was checked
# against (2,866 staged files, zero differences) has been removed; it is in the
# git history at tag v0.1.0.
#
# No root, no chroot, no namespaces; transport is a plain HTTP GET on port 80.
#
# libc6 is deliberately skipped: the rootfs already carries a libc whose optional
# syscalls have been rewritten, and replacing it would undo that.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)

SUITE=${SUITE:-noble}
SUITE_UPDATES=${SUITE_UPDATES:-$SUITE-updates}
ARCH=${ARCH:-arm64}
PKGS=${PKGS:-"gcc make binutils libc6-dev dash sed grep tar gzip findutils diffutils"}
STAGE=${STAGE:-/tmp/nr-toolchain/root}
CACHE=${CACHE:-/tmp/nr-toolchain/deb}
SKIP=${SKIP:-libc6}
ADB=${ADB:-adb}
SERIAL=${SERIAL:-}
APP=${APP:-com.linuxhub.probe}
GUEST_ROOT=${GUEST_ROOT:-files/nrfull/root}

cargo build --release -q -p nr-tools --manifest-path "$ROOT/runtime/Cargo.toml"
TOOLS=$ROOT/runtime/target/release/nr-tools

set -- provision \
    --suite "$SUITE" --suite-updates "$SUITE_UPDATES" --arch "$ARCH" \
    --packages "$PKGS" --stage "$STAGE" --cache "$CACHE" --skip "$SKIP" \
    --app "$APP" --guest-root "$GUEST_ROOT" --adb "$ADB"
if [ -n "$SERIAL" ]; then set -- "$@" --serial "$SERIAL"; fi
if [ -n "${NO_PUSH:-}" ]; then set -- "$@" --no-push; fi

exec "$TOOLS" "$@"
