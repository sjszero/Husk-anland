#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")" && pwd -P)
UPSTREAM_REPO=${XLORIE_UPSTREAM_REPO:-https://github.com/termux/termux-x11.git}
UPSTREAM_COMMIT=${XLORIE_UPSTREAM_COMMIT:-e382844d468881c3a3460291cb02d4eaaa84c024}
BUILD_ROOT=${XLORIE_BUILD_DIR:-/tmp/husk-xlorie-build}
NDK=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-$HOME/Library/Android/sdk/ndk/29.0.14206865}}
CMAKE=${CMAKE:-cmake}
JOBS=${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '4')}

sha256_file() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        sha256sum "$1" | awk '{print $1}'
    fi
}

[ -f "$NDK/build/cmake/android.toolchain.cmake" ] || {
    echo "Android NDK not found: $NDK" >&2
    exit 2
}
command -v git >/dev/null || { echo "git is required" >&2; exit 2; }
command -v "$CMAKE" >/dev/null || { echo "cmake is required" >&2; exit 2; }
command -v ninja >/dev/null || { echo "ninja is required" >&2; exit 2; }
if [ -z "${LLVM_STRIP:-}" ]; then
    for candidate in "$NDK"/toolchains/llvm/prebuilt/*/bin/llvm-strip; do
        [ -x "$candidate" ] && LLVM_STRIP=$candidate && break
    done
fi
[ -n "${LLVM_STRIP:-}" ] && [ -x "$LLVM_STRIP" ] || {
    echo "llvm-strip not found in Android NDK: $NDK" >&2
    exit 2
}

verify_and_apply() {
    tree=$1
    name=$2
    expected=$3
    patch="$ROOT/patches/$name"
    actual=$(sha256_file "$patch")
    [ "$actual" = "$expected" ] || {
        echo "Xlorie patch hash mismatch: $name $actual" >&2
        exit 2
    }
    git -C "$tree" apply --check "$patch"
    git -C "$tree" apply "$patch"
}

mkdir -p "$BUILD_ROOT"
SRC="$BUILD_ROOT/termux-x11"
OUT="$BUILD_ROOT/cmake-arm64"
if [ -e "$SRC" ] && [ ! -d "$SRC/.git" ]; then
    echo "Xlorie build source exists but is not a git checkout: $SRC" >&2
    exit 2
fi
if [ ! -d "$SRC/.git" ]; then
    git clone --filter=blob:none "$UPSTREAM_REPO" "$SRC"
else
    actual_remote=$(git -C "$SRC" config --get remote.origin.url)
    [ "$actual_remote" = "$UPSTREAM_REPO" ] || {
        echo "Refusing to reset unexpected Xlorie checkout: $actual_remote" >&2
        exit 2
    }
fi
git -C "$SRC" fetch --tags origin
git -C "$SRC" checkout --detach "$UPSTREAM_COMMIT"
git -C "$SRC" reset --hard "$UPSTREAM_COMMIT"
git -C "$SRC" clean -fdx
git -C "$SRC" submodule sync --recursive
git -C "$SRC" submodule update --init --recursive --force
git -C "$SRC" submodule foreach --recursive 'git reset --hard && git clean -fdx'

# Reusing a CMake directory can silently retain objects from older patches.
rm -rf "$OUT"

verify_and_apply "$SRC" 0001-husk-embedded-x11.patch \
    3ca7797cd79f64ff60fd0eb10e312044bddec97e95dc107dd9e28e695415504a
verify_and_apply "$SRC/lorie/src/main/cpp/libepoxy" 0002-libepoxy-android.patch \
    7ba07fbb866977622b1f072b805b29dcaacdf208d6eb6d8cc761ea4f61b4c59d
verify_and_apply "$SRC/lorie/src/main/cpp/libx11" 0003-libx11-android.patch \
    5baf23495b11c8bec9e184d2686202dc731b5cb9ffc671b4b5463d69d1ed5f4f
verify_and_apply "$SRC/lorie/src/main/cpp/libxkbfile" 0004-libxkbfile-android.patch \
    b336a927917dfca924fc48e46c8f18653ea630163de47e0edfddcdb3adabb4cf
verify_and_apply "$SRC/lorie/src/main/cpp/libxtrans" 0005-libxtrans-android.patch \
    aa3442ab3dc0c871cd410662dfd27ba79192195bbeda4059a02e6042d31062a1
verify_and_apply "$SRC/lorie/src/main/cpp/pixman" 0006-pixman-android.patch \
    b6e4fe1d15c1e1155d123dc95435cccd7aa3594bceea69789e8e250634c56904
verify_and_apply "$SRC/lorie/src/main/cpp/xkbcomp" 0007-xkbcomp-android.patch \
    9646b19922e8d4583aee24e264d815e14bbb2b6f1667311a844db09aefe987b6
verify_and_apply "$SRC/lorie/src/main/cpp/xserver" 0008-xserver-android.patch \
    c51cf76c6cfe7bd38ed346136e582c93f399749cf18abdacb1c04991db0d3479

REPRO_FLAGS="-O3 -g0 -ffile-prefix-map=$SRC=/husk-xlorie -fdebug-prefix-map=$SRC=/husk-xlorie -fmacro-prefix-map=$SRC=/husk-xlorie -fdebug-compilation-dir=/husk-xlorie-build"
"$CMAKE" -S "$SRC/lorie/src/main/cpp" -B "$OUT" -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$NDK/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI=arm64-v8a \
    -DANDROID_PLATFORM=android-26 \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_FLAGS="$REPRO_FLAGS" \
    -DCMAKE_CXX_FLAGS="$REPRO_FLAGS" \
    -DCMAKE_SHARED_LINKER_FLAGS="-Wl,--build-id=none"
"$CMAKE" --build "$OUT" --target Xlorie --parallel "$JOBS"

BIN="$OUT/libXlorie.so"
[ -f "$BIN" ] || { echo "build output missing: $BIN" >&2; exit 1; }
"$LLVM_STRIP" --strip-all "$BIN"
mkdir -p "$ROOT/arm64-v8a"
cp "$BIN" "$ROOT/arm64-v8a/libXlorie.so"
echo "installed: $ROOT/arm64-v8a/libXlorie.so"
echo "sha256: $(sha256_file "$ROOT/arm64-v8a/libXlorie.so")"
