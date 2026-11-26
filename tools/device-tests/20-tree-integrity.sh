#!/bin/sh
# 20 · 这棵树是不是"装完整的树"
#
# 起因：2026-11-26 的桌面树 `rootfs-broken-20261126` 有 945 个包、能跑 Mesa 和 GNOME，
# 但整棵没有 `etc/husk/`、没有 `usr/local/share/linuxhub/`、没有安装器写的 profile.d
# 那几份 —— 它是一个"只解了 Ubuntu 包"的树，Husk 的安装层从来没进去过。这种树不会
# 报错：GPU 静默掉回 llvmpipe（evidence 291）、gnome-shell 因为连不上系统总线在
# main.js 里抛异常（evidence 191）、桌面进"安全模式"。所以缺件必须能被一条命令查出来。
#
#   DEV=UKHMMB8PYTGECMNV ROOTFS=files/.../rootfs ./20-tree-integrity.sh
#   DEV=... ROOTFS=files/.../rootfs-broken-20261126 EXPECT=desktop ./20-tree-integrity.sh
#
# EXPECT=base（默认）只要求 Linux 用户态 + GPU 层；EXPECT=desktop 另外要求桌面会话
# 需要的系统总线配置与语言包。
set -u
DEV=${DEV:?usage: DEV=<serial> [ROOTFS=...] [EXPECT=base|desktop] $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs}
EXPECT=${EXPECT:-base}
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
have() { adb -s "$DEV" shell "run-as $PKG sh -c 'test -e $ROOTFS/$1 && echo yes'" 2>/dev/null | grep -q yes; }
read_() { adb -s "$DEV" shell "run-as $PKG sh -c 'cat $ROOTFS/$1 2>/dev/null'" 2>/dev/null; }

echo "== core userspace =="
if have usr/bin/bash; then ok "usr/bin/bash present"; else bad "usr/bin/bash missing: not a Linux userspace"; fi
if have etc/os-release; then
    ok "etc/os-release: $(read_ etc/os-release | sed -n 's/^PRETTY_NAME=//p' | tr -d '\"')"
else
    bad "etc/os-release missing"
fi

echo "== husk install layer =="
if have etc/husk/gpu-env; then ok "etc/husk/gpu-env present"; else bad "etc/husk/gpu-env missing: loader injects nothing, GPU falls back to llvmpipe silently"; fi
for f in etc/profile.d/husk-gpu.sh etc/profile.d/husk-locale.sh; do
    if have "$f"; then ok "$f present"; else bad "$f missing: login shells get no GPU/locale defaults"; fi
done
if have usr/local/share/linuxhub/husk-wallpaper-scale.js; then
    ok "usr/local/share/linuxhub/husk-wallpaper-scale.js present"
else
    bad "usr/local/share/linuxhub/husk-wallpaper-scale.js missing"
fi

echo "== gpu layer consistent with this device =="
drv=$(adb -s "$DEV" shell "run-as $PKG sh -c 'ls $ROOTFS/usr/lib/aarch64-linux-gnu/libvulkan_panfrost*.so 2>/dev/null | head -1'" | tr -d '\r')
envf=$(read_ etc/husk/gpu-env)
case "$envf" in
    "@GPU_FAMILY=mali-kbase"*) family=mali-kbase ;;
    "@GPU_FAMILY=adreno-kgsl"*) family=adreno-kgsl ;;
    "@GPU_FAMILY="*) family=$(printf '%s\n' "$envf" | head -1 | cut -d= -f2) ;;
    *) family=none ;;
esac
echo "  STATE family=${family} driver=${drv:-none}"
if [ -n "$drv" ]; then
    case "$family" in
        mali-kbase|adreno-kgsl) ok "tree has a GPU driver and gpu-env names a hardware family" ;;
        *) bad "PanVK/freedreno driver present but gpu-env family is '${family}'" ;;
    esac
    icd=$(printf '%s\n' "$envf" | sed -n 's/^VK_ICD_FILENAMES=//p' | head -1)
    if [ -n "$icd" ] && have "${icd#/}"; then
        ok "ICD manifest exists in the tree ($icd)"
    else
        bad "ICD manifest missing or unset (got '${icd:-<none>}')"
    fi
else
    ok "no vendor Vulkan driver in this tree (base tree, expected)"
fi

echo "== desktop session requirements =="
if [ "$EXPECT" = desktop ]; then
    if have usr/local/share/linuxhub/linuxhub-system-bus.conf; then
        ok "linuxhub-system-bus.conf present"
    else
        bad "linuxhub-system-bus.conf missing: gnome-shell aborts on 'Couldn't connect to system bus'"
    fi
    n=$(adb -s "$DEV" shell "run-as $PKG sh -c 'ls $ROOTFS/usr/share/locale-langpack/zh_CN/LC_MESSAGES 2>/dev/null | wc -l'" | tr -d '\r')
    if [ "${n:-0}" -ge 400 ]; then ok "zh_CN langpack: $n .mo files"; else bad "zh_CN langpack only ${n:-0} .mo files"; fi
    if have usr/bin/dbus-daemon; then ok "dbus-daemon present"; else bad "dbus-daemon missing"; fi
else
    echo "  SKIP  desktop-only checks (EXPECT=base)"
fi

echo "TREE_INTEGRITY PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
