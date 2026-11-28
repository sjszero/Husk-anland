#!/bin/sh
# Release gate for the vendored Mali kbase PanVK snapshot.
#
# A mismatched manifest or a truncated gzip does not fail at build time by
# itself: the release tar still packs, the installer reports success, and only
# the first GL/Vulkan program silently falls back to llvmpipe (or SIGSEGVs).
# This script checks the bytes and the guest-relative ICD contract before dist.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
V="$ROOT/vendor/panvk/fristonerr-1.2.0-mali-g77"
DRIVER_GZ="$V/libvulkan_panfrost_frist370_execva.so.gz"
ICD="$V/husk_panvk_guest.json"
INSTALL="$ROOT/tools/guest/install-husk.sh"
WANT_GZ=47d11cd2fa0cfc36a7d48028c4d81d0a1a920b9b772c5583fc1e8c0c7b84140b
WANT_SO=5e06324783ec7947d48782441fd1cac23953c90bd0b31663f98f08b22344c1c3
# 0.0.37 起驱动由源码构建（热身=配置、blit=单三角形），安装器只核 SHA-256，
# 不再按偏移改字节。下面的断言就是"别再偷偷改字节"的看门人。
WARMUP_META="$ROOT/docs/design/patches/fristonerr-kbase-warmup-skip.meta"
WARMUP_PATCH="$ROOT/docs/design/patches/fristonerr-kbase-warmup-skip.patch"
BLIT_PATCH="$ROOT/docs/design/patches/fristonerr-blit-single-triangle.patch"
TMP=${TMPDIR:-/tmp}/husk-panvk-audit.$$
TMP2=${TMPDIR:-/tmp}/husk-panvk-audit-patched.$$
trap 'rm -f "$TMP" "$TMP2"' EXIT HUP INT TERM

fail() { echo "panvk bundle: FAIL: $*" >&2; exit 1; }
[ -f "$DRIVER_GZ" ] || fail "missing $DRIVER_GZ"
[ -f "$ICD" ] || fail "missing $ICD"
[ -f "$V/LICENSE-Mesa.txt" ] || fail "missing Mesa license"
[ -f "$V/PROVENANCE.md" ] || fail "missing provenance"
[ -f "$INSTALL" ] || fail "missing installer"

have=$(sha256sum "$DRIVER_GZ" | cut -d' ' -f1)
[ "$have" = "$WANT_GZ" ] || fail "compressed driver sha256 $have != $WANT_GZ"

gzip -dc "$DRIVER_GZ" > "$TMP" || fail "driver gzip is corrupt"
have=$(sha256sum "$TMP" | cut -d' ' -f1)
[ "$have" = "$WANT_SO" ] || fail "decompressed driver sha256 $have != $WANT_SO"

grep -q '"library_path"[[:space:]]*:[[:space:]]*"/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so"' "$ICD" \
    || fail "ICD library_path is not the guest-relative driver path"
grep -q '"api_version"[[:space:]]*:[[:space:]]*"1.3.0"' "$ICD" \
    || fail "ICD api_version is not 1.3.0"
grep -Fq '_panvk_driver=usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so' "$INSTALL" \
    || fail "installer does not use the Husk-owned PanVK path"
grep -Fq 'libvulkan_panfrost_husk.so' "$INSTALL" \
    || fail "installer driver search does not prefer the Husk-owned PanVK path"
[ -f "$WARMUP_META" ] || fail "missing kbase warmup patch record"
[ -f "$WARMUP_PATCH" ] || fail "missing kbase warmup source patch"
grep -q 'binary_patch_offset 0x53a570' "$WARMUP_META" || fail "warmup record lost its offset"
grep -Fq "$WANT_SO" "$WARMUP_META" || fail "warmup record has the wrong driver sha256"
grep -Fq "$WANT_SO" "$INSTALL" || fail "installer does not pin the driver sha256"
[ -f "$BLIT_PATCH" ] || fail "missing vk_meta blit source patch"
grep -q 'vk_meta_draw_rects.c' "$BLIT_PATCH" || fail "blit patch does not touch vk_meta_draw_rects.c"
grep -q 'create_fullscreen_triangle_buffer' "$BLIT_PATCH" || fail "blit patch is not the single-triangle fix"
if grep -q '_panvk_warmup_off' "$INSTALL"; then
    fail "installer still byte-patches the driver; the driver is source-built now"
fi
grep -q 'PANVK_KBASE_WARMUP_MS' "$INSTALL" || fail "installer does not document the warmup knob"
grep -q 'PANVK_KBASE_WARMUP_MS' "$V/PROVENANCE.md" || fail "provenance does not document the warmup knob"
grep -q 'fristonerr-blit-single-triangle.patch' "$V/PROVENANCE.md" || fail "provenance does not list the blit patch"

# 0.0.37 · 过期 ICD / 驱动副本清理（evidence 357 §7 / defect H43）。
# 发布包里只能有一颗驱动；安装器必须清掉实验期的 nosleep 清单与旧 .so 副本，
# 否则驱动一升级，旧副本会先被 ICD 播出的路径选中，而且不会有任何报错。
for _junk in "$V"/*nosleep*; do
    [ -e "$_junk" ] && fail "vendor bundle still carries $(basename "$_junk")"
done
_so_count=$(ls "$V"/*.so 2>/dev/null | wc -l | tr -d ' ')
[ "$_so_count" = 0 ] || fail "vendor bundle carries $_so_count unpacked .so file(s); ship the .gz only"
grep -q 'nosleep' "$INSTALL" || fail "installer does not clean up nosleep ICD manifests"
grep -q 'removed stale GPU artifact' "$INSTALL" || fail "installer does not remove stale GPU artifacts"
grep -q 'libvulkan_panfrost_x11.so' "$INSTALL" || fail "installer does not treat the x11 driver copy as stale"

# 独立复算：解压出来的 .so 必须就是安装器 pin 的那一份，且没有二次改写的迹象。
have=$(sha256sum "$TMP" | cut -d' ' -f1)
[ "$have" = "$WANT_SO" ] || fail "driver sha256 $have != $WANT_SO"

grep -q 'FristOneRR' "$V/PROVENANCE.md" || fail "provenance has no upstream identity"
grep -q '0c9dce35094dbe89c412b79bf57c6067c7b97e59' "$V/PROVENANCE.md" \
    || fail "provenance has no pinned source commit"
grep -q 'Permission is hereby granted' "$V/LICENSE-Mesa.txt" \
    || fail "Mesa license text is missing"

echo "panvk bundle: PASS ($WANT_GZ, driver $WANT_SO)"
