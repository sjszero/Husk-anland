#!/bin/sh
# Build the guest helper binaries that build-init.sh / build-logd.sh do not
# cover: husk-usermgr, husk-ldconfig, husk-rd, husk-su.
#
# Over-aligned to 64 KiB: that is the only p_align that is legal on a 4K, 16K and
# 64K page kernel at the same time. glibc refuses a PT_LOAD whose p_align is not
# a multiple of the running page size, so a 16K build cannot be loaded on a 64K
# device (and the other way round). nr-tools check-elf verifies the result.
#
# These three are dynamically linked (libc; husk-rd also dlopens libgio), so the
# link needs a real aarch64 glibc sysroot - crt1.o/crti.o/crtn.o, the libc.so
# linker script (which pulls in libc_nonshared.a) and the runtime .so files.
# tools/device-tests/provision-toolchain.sh stages one at /tmp/nr-toolchain/root
# (gcc, binutils, make, libc6-dev; libc6 itself is deliberately skipped because
# the guest's own patched libc must survive, so copy the handful of .so.6 files
# and the loader in from a libc6 arm64 package, and see tools/README if the
# libm.so script mentions libmvec - it is not needed and not shipped).
#
# The NDK's clang has to be the linker *driver*, not rust-lld directly: only the
# C driver adds crt1.o/crti.o/crtn.o. Linking with rust-lld alone silently
# produces e_entry = 0, i.e. a binary whose first instruction fetch faults
# (measured: every helper built that way segfaulted before main, with or without
# the Husk loader).
set -e
cd "$(dirname "$0")/../../runtime"
NDK=${NDK:-/Users/l/Library/Android/sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/darwin-x86_64}
SYSROOT=${SYSROOT:-/tmp/nr-toolchain/root}
# Accept both spellings: build-embedded-loader.sh wants <prebuilt>/bin because it
# calls $NDK/clang, this script wants <prebuilt> because it calls $NDK/bin/clang.
case "$NDK" in */bin) NDK=${NDK%/bin} ;; esac
[ -x "$NDK/bin/clang" ] || { echo "$0: no clang at $NDK/bin/clang" >&2; exit 1; }
for f in usr/lib/aarch64-linux-gnu/crt1.o usr/lib/aarch64-linux-gnu/libc.so lib/ld-linux-aarch64.so.1; do
  [ -e "$SYSROOT/$f" ] || { echo "$0: incomplete sysroot: $SYSROOT/$f missing" >&2; exit 1; }
done
FLAGS="-C linker=$NDK/bin/clang \
       -C link-arg=--target=aarch64-linux-gnu \
       -C link-arg=--sysroot=$SYSROOT \
       -C link-arg=-fuse-ld=$NDK/bin/ld.lld \
       -C link-arg=-Wl,--dynamic-linker=/lib/ld-linux-aarch64.so.1 \
       -C link-arg=-Wl,-z,max-page-size=65536"
for pkg in usermgr ldconfig husk-rd husk-su; do
  RUSTFLAGS="$FLAGS" cargo build --release -p "$pkg" --target aarch64-unknown-linux-gnu
done
for b in husk-usermgr husk-ldconfig husk-rd husk-su; do
  printf '%s  ' "$b"
  "$NDK/bin/llvm-readelf" -h "target/aarch64-unknown-linux-gnu/release/$b" | awk -F: '/Entry point/{print "entry" $2}'
done
ls -l target/aarch64-unknown-linux-gnu/release/husk-usermgr \
      target/aarch64-unknown-linux-gnu/release/husk-ldconfig \
      target/aarch64-unknown-linux-gnu/release/husk-rd \
      target/aarch64-unknown-linux-gnu/release/husk-su
