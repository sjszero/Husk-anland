#!/bin/sh
# Build husk-init, the guest service supervisor.
#
# no_std + static + no libc on purpose (see the header of
# runtime/crates/husk-init/src/main.rs): a supervisor has to keep running while
# the container is torn down, so it cannot depend on the loader or the shim.
# Over-aligned to 16 KiB so the same binary is legal on a 16K-page device.
set -e
cd "$(dirname "$0")/../../runtime"
RUSTFLAGS="-C link-arg=-nostdlib -C link-arg=-static \
           -C link-arg=-z -C link-arg=max-page-size=65536" \
  cargo build --release -p husk-init --target aarch64-unknown-linux-gnu
OUT=${OUT:-target/aarch64-unknown-linux-gnu/release/husk-init}
ls -l "$OUT"
