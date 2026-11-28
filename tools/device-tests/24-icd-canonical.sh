#!/bin/sh
# 24 · 安装树里只能有一颗 PanVK 驱动、一个 canonical ICD（defect H43 / evidence 357 §7）。
#
# 为什么这是门禁而不是洁癖：ICD 清单里的 library_path 决定选哪颗 .so。旧副本和
# 实验清单留在树上时，今天两份字节相同所以看不出问题；驱动一升级，旧副本可能先赢，
# 而表现出来只是"新驱动没生效"，不报任何错。
#
#   DEV=<adb serial> PKG=com.linuxhub.myth.husk sh tools/device-tests/24-icd-canonical.sh
set -u
DEV=${DEV:?usage: DEV=<adb serial> [PKG=...] [ROOTFS=...] $0}
PKG=${PKG:-com.linuxhub.myth.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
WANT_SHA=5e06324783ec7947d48782441fd1cac23953c90bd0b31663f98f08b22344c1c3
CANON=/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so
PASS=0; FAIL=0; SKIP=0
ok()   { PASS=$((PASS+1)); echo "  PASS  $*"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL  $*"; }
sh_()  { adb -s "$DEV" shell "run-as $PKG sh -c '$*'" 2>&1; }

echo "== T1/T2 every husk ICD manifest points at the canonical driver =="
icds=$(DEV=$DEV PKG=$PKG ROOTFS=$ROOTFS LD=$LD sh "$HERE/run-guest.sh" t-24-icd.sh 2>&1 | grep '^ICD ')
echo "$icds" | sed 's/^/    /'
n_icd=0; n_ok=0
while read -r tag name path; do
    [ "$tag" = ICD ] || continue
    n_icd=$((n_icd+1))
    if [ "$path" = "$CANON" ]; then n_ok=$((n_ok+1)); ok "$name -> $path"
    else bad "$name -> [$path] (want $CANON)"; fi
done <<EOF_ICDS
$icds
EOF_ICDS
[ "$n_icd" -ge 1 ] && ok "$n_icd husk manifest(s) found" || bad "no husk_panvk*.json manifest found at all"

echo "== T3 no experimental manifests left =="
n=$(sh_ "ls $ROOTFS/usr/share/vulkan/icd.d/ | grep -c nosleep" | tr -d '\r')
[ "$n" = 0 ] && ok "no *nosleep* ICD manifest" || bad "$n nosleep manifest(s) still installed"

echo "== T4 no stale driver copies on the side =="
for d in \
    usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk_nosleep.so \
    usr/lib/aarch64-linux-gnu/libvulkan_panfrost_x11.so \
    usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370_execva.so \
    usr/lib/aarch64-linux-gnu/libvulkan_panfrost_frist370.so \
    usr/lib/aarch64-linux-gnu/libvulkan_panfrost_g77.so \
    usr/lib/aarch64-linux-gnu/.libvulkan_panfrost_husk.so.bak-orig
do
    if sh_ "[ -e $ROOTFS/$d ] && echo yes" | grep -q yes; then bad "stale copy present: $d"; else ok "absent: ${d##*/}"; fi
done

echo "== T5 installed driver is the warmup-patched build =="
have=$(sh_ "sha256sum $ROOTFS/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so" | awk '{print $1}' | tr -d '\r')
[ "$have" = "$WANT_SHA" ] && ok "driver sha256 $have" || bad "driver sha256 $have != $WANT_SHA"

echo "== T6 gpu-env pins the canonical ICD and zink =="
env_=$(sh_ "cat $ROOTFS/etc/husk/gpu-env")
echo "$env_" | grep -q '^VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/husk_panvk_guest.json' \
    && ok "VK_ICD_FILENAMES -> canonical" || bad "gpu-env does not pin the canonical ICD"
echo "$env_" | grep -q '^!MESA_LOADER_DRIVER_OVERRIDE=zink' \
    && ok "MESA_LOADER_DRIVER_OVERRIDE=zink (forced)" || bad "gpu-env lost the forced zink override"

echo "== T7 runtime: loader scan + legacy manifest name both still pick a working Vulkan =="
# 发行包不带 GL/Vulkan 探针（`vulkan-probe` 只存在于桌面树）。树里没有探针时这一段
# 只能 SKIP，不能算 PASS，也不能算 FAIL——它测的是"探针能建实例"，不是装箱单。
if ! sh_ "[ -x $ROOTFS/usr/bin/vulkan-probe ] && echo yes" | grep -q yes; then
    SKIP=$((SKIP+1))
    echo "  SKIP  this tree has no usr/bin/vulkan-probe (release trees do not ship GL probes)"
else
out=$(DEV=$DEV PKG=$PKG ROOTFS=$ROOTFS LD=$LD sh "$HERE/run-guest.sh" t-24-vk.sh 2>&1)
echo "$out" | sed 's/^/    /'
echo "$out" | grep -q 'vk|instance|PASS' && ok "vulkan-probe instance created (loader scan)" || bad "loader scan path failed"
echo "$out" | grep -c 'vk|instance|PASS' | grep -q '^2$' && ok "legacy manifest name also creates an instance (H40 host pin safe)" \
    || bad "legacy manifest name does not create an instance"

fi

echo "ICD_CANONICAL PASS=$PASS FAIL=$FAIL SKIP=${SKIP:-0}"
[ "$FAIL" = 0 ] || exit 1
