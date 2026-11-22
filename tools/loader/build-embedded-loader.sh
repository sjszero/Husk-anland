#!/bin/sh
# Build the loader-embedded path shim, end to end, from the pristine inputs.
#
#   path-shim (Rust, no_std, zero DT_NEEDED)
#     -> 4K-aligned ELF segments
#     -> init-hook.S  (_dl_init hook, 176 bytes, no relocations)
#     -> nr-tools embed  (one extra R|E PT_LOAD in ld.so, all relocations baked)
#
# The ELF surgery is `nr-tools` (Rust, runtime/crates/nr-tools) and it is the only
# implementation. The Python scripts it replaced were kept as an oracle until the
# port reproduced them byte for byte; they are in the git history at tag v0.1.0.
#
# Prints the numbers nr-tools asserted/derived. Nothing here is a hard-coded
# offset: the tool checks the loader it is given and refuses to patch a different
# one.
set -e
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
NDK=${NDK:?set NDK to your NDK toolchain bin dir, e.g. $ANDROID_HOME/ndk/<ver>/toolchains/llvm/prebuilt/<host>/bin}
OUT=${OUT:-$ROOT/tools/probe-app/app/src/main/jniLibs/arm64-v8a}
# The base loader must come from the rootfs that will actually run: ld.so and
# libc are a matched pair, and the *_dl_init/scan sites are version-checked.
# ld-noble2404-ld.so is the untouched loader of the Ubuntu 24.04 rootfs used on
# the test device; override with LOADER=<path> for another rootfs.
LOADER=${LOADER:-ld-noble2404-ld.so}

cd "$ROOT/runtime"
RUSTFLAGS="-C link-arg=-z -C link-arg=max-page-size=4096 \
           -C link-arg=-z -C link-arg=separate-loadable-segments" \
CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
CARGO_PROFILE_RELEASE_LTO=fat \
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
CARGO_PROFILE_RELEASE_STRIP=symbols \
  cargo build --release -p path-shim --target aarch64-unknown-linux-gnu \
  --manifest-path "$ROOT/runtime/Cargo.toml"

cargo build --release -p nr-tools --manifest-path "$ROOT/runtime/Cargo.toml"

SHIM=$ROOT/runtime/target/aarch64-unknown-linux-gnu/release/libpath_shim.so
TOOLS=$ROOT/runtime/target/release/nr-tools
cd "$ROOT/tools/loader"

scan() { "$TOOLS" scan-syscalls "$@"; }
embed() { "$TOOLS" embed "$@"; }

$NDK/clang -target aarch64-linux-gnu -c init-hook.S -o init-hook.o
$NDK/llvm-objcopy -O binary init-hook.o init-hook.bin

# Optional-accelerator `svc` sites are located by pattern, not by offset, and
# every expectation below is an assertion: the glibc version, and how many sites
# of each kind the image must have. A different glibc build fails here instead of
# producing a loader with the wrong instructions removed.
scan patch "$LOADER" ld-stage1.so \
    --version 2.35 --expect set_robust_list=1 --expect rseq=1
embed "$SHIM" ld-stage1.so init-hook.bin ld-embedded.so

# The guest libc carries the same calls (its own pthread path and _Fork). It
# lives in the rootfs, not in this repo, so the patched file is produced here and
# installed into the rootfs by whoever provisions it. Point LIBC at a pristine
# copy to rebuild it.
if [ -n "${LIBC:-}" ]; then
    scan patch "$LIBC" libc-nop.so.6 \
        --version 2.39 --expect set_robust_list=2 --expect rseq=1
fi

mkdir -p "$OUT"
cp ld-embedded.so "$OUT/libld.so"
ls -l "$OUT/libld.so"
