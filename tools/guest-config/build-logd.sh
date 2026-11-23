#!/bin/sh
# Build husk-logd, the /dev/log sink, for the guest.
#
# no_std + static + no libc on purpose: see the header of
# runtime/crates/logd/src/main.rs. The result is ~12 KB and needs nothing from
# the rootfs, which is what lets it create the socket before anything else runs.
set -e
cd "$(dirname "$0")/../../runtime"
RUSTFLAGS="-C link-arg=-nostdlib -C link-arg=-static \
           -C link-arg=-z -C link-arg=max-page-size=65536" \
  cargo build --release -p logd --target aarch64-unknown-linux-gnu
OUT=${OUT:-target/aarch64-unknown-linux-gnu/release/husk-logd}
ls -l "$OUT"
