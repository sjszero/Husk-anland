#!/bin/sh
# Re-apply the runtime's libc patches to a rootfs on the device.
#
# Why this exists: the guest libc is not stock. Two transforms are baked into it
# (tools/loader/build-embedded-loader.sh documents the chain):
#
#   1. slot interposition - the calls libc makes *inside itself* (nss_files'
#      fopen("/etc/passwd"), _IO_file_open, opendir, realpath) have no GOT entry
#      to rewrite, so the shim gets a function-pointer slot page instead;
#   2. clone3 -> ENOSYS - Android's seccomp policy refuses clone3, and glibc only
#      takes its legacy clone fallback on ENOSYS.
#
# Any `apt upgrade` of libc6 silently replaces that file with the stock one.
# Measured 2026-11-24: after installing build-essential the guest's
# `getent shadow` returned nothing (nss_files opened the *host* /etc/shadow),
# 2.0's sudo-access step failed, and the desktop would not start.
#
#   ./install-libc-patch.sh          # patch whatever libc the rootfs has now
#   LIBC=/path/to/libc.so.6 ./install-libc-patch.sh
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
. "$HERE/../bench/bench.env"
PKGDIR=files/linux/distributions/ubuntu-24.04/rootfs
TMP=$(mktemp -d /tmp/husk-libc-XXXXXX)

TOOLS=$ROOT/runtime/target/release/nr-tools
[ -x "$TOOLS" ] || cargo build --release -q -p nr-tools --manifest-path "$ROOT/runtime/Cargo.toml"

if [ -n "${LIBC:-}" ]; then
  cp "$LIBC" "$TMP/libc.so.6"
else
  adb -s "$DEV" shell "run-as $PKG sh -c 'cat $PKGDIR/usr/lib/aarch64-linux-gnu/libc.so.6'" > "$TMP/libc.so.6"
fi

"$TOOLS" libc-interpose "$TMP/libc.so.6" "$TMP/libc-slots.so.6" "$ROOT/tools/loader/libc-interpose.txt" >/dev/null
"$TOOLS" scan-syscalls patch "$TMP/libc-slots.so.6" "$TMP/libc-nop.so.6" \
  --version 2.39 --expect set_robust_list=2 --expect rseq=1 --expect clone3=1

adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $PKGDIR/usr/lib/aarch64-linux-gnu/libc.so.6.husk'" < "$TMP/libc-nop.so.6"
adb -s "$DEV" shell "run-as $PKG sh -c '
  R=$PKGDIR/usr/lib/aarch64-linux-gnu
  mkdir -p $PKGDIR/usr/share/husk
  if [ ! -f \$R/libc.so.6.stock ]; then cp \$R/libc.so.6 \$R/libc.so.6.stock; fi
  mv \$R/libc.so.6.husk \$R/libc.so.6
  chmod 755 \$R/libc.so.6
  sha256sum \$R/libc.so.6'"
cp "$TMP/libc-nop.so.6" "$ROOT/tools/loader/libc-nop.so.6"
cp "$TMP/libc-slots.so.6" "$ROOT/tools/loader/libc-slots.so.6"
echo "ok: libc patched and installed; repo artifacts updated"
