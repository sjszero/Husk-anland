#!/bin/sh
# 18 · GPU 默认值到底是"进程一启动就有"，还是只有桌面会话脚本给的那一份？
#
# 为什么单列一条：`husk-session.sh`/`husk-desktop.sh` 会在自己这一层 export
# `VK_ICD_FILENAMES` + zink，桌面与它 fork 出来的应用都吃得到——看起来"GPU 全局"，
# 但只要进程不是从那个会话里出来的（cron、systemd user unit、脚本里的 env -i、
# `sh -c` 里再 exec 一次），就掉回 llvmpipe。真正的全局靠加载器读
# `/etc/husk/gpu-env` 填空（runtime/crates/path-shim/src/gpuenv.rs）。
#
# 这个脚本只量这件事：同一棵树、同一个探针、不带任何 GPU 环境变量，
# 谁给的值生效；再用两个开关做反向控制。
#
#   DEV=UKHMMB8PYTGECMNV ROOTFS=files/rootfs-full \
#   LD=files/linux/libld.so ./18-gpu-loader-defaults.sh
#
# 退出码 0 = 加载器注入生效（树里有 PanVK 驱动时）；2 = 树里没有驱动，只验软件路径。
set -u
DEV=${DEV:?usage: DEV=<serial> [ROOTFS=...] [LD=...] $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
GL_PROBE=${GL_PROBE:-$HERE/../../runtime/target/aarch64-unknown-linux-gnu/release/gl-probe}
PASS=0; FAIL=0; SKIP=0
ok()   { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
skip() { SKIP=$((SKIP+1)); echo "  SKIP  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

push() { # push <local> <guest-relative-in-tree>
    adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $ROOTFS/$2'" < "$1"
}

renderer() { # renderer <tag> [env assignments...]
    tag=$1; shift
    out=$(adb_ "run-as $PKG sh $ROOTFS/tmp/husk-exec.sh $ROOTFS $LD usr/bin/env $* ./root/gl-probe readback 16x16" 2>&1)
    printf '%s\n' "$out" | sed -n 's/^renderer=//p' | head -1
}

echo "== tree =="
if adb_ "run-as $PKG sh -c 'ls $ROOTFS/usr/lib/aarch64-linux-gnu/libvulkan_panfrost*.so 2>/dev/null'" | grep -q libvulkan_panfrost; then
    have_driver=true; ok "tree carries a PanVK driver"
else
    have_driver=false; skip "tree carries no PanVK driver (software-only run)"
fi

if [ "$have_driver" = true ]; then
    env_file=$(adb_ "run-as $PKG sh -c 'cat $ROOTFS/etc/husk/gpu-env 2>/dev/null'")
    if [ -n "$env_file" ]; then
        ok "tree carries etc/husk/gpu-env"
    else
        bad "PanVK driver present but etc/husk/gpu-env missing: every non-session process falls back to llvmpipe (write it with tools/guest-config/husk-gpu-defaults.sh or re-run the installer)"
    fi
    case "$env_file" in
        "@GPU_FAMILY=mali-kbase"*) ok "gpu-env family line matches this device (mali-kbase)" ;;
        "") ;;
        *) bad "gpu-env first line is not @GPU_FAMILY=mali-kbase (got: $(printf '%s\n' "$env_file" | head -1))" ;;
    esac
    # ICD 必须指向树里真实存在的驱动，且是 guest 相对路径。
    icd=$(printf '%s\n' "$env_file" | sed -n 's/^VK_ICD_FILENAMES=//p' | head -1)
    if [ -n "$icd" ]; then
        if adb_ "run-as $PKG sh -c 'test -r $ROOTFS$icd && echo yes'" | grep -q yes; then
            ok "VK_ICD_FILENAMES points at a file inside the tree ($icd)"
        else
            bad "VK_ICD_FILENAMES=$icd does not exist in the tree"
        fi
        drv=$(adb_ "run-as $PKG sh -c 'sed -n \"s/.*library_path\\\"[ ]*:[ ]*\\\"\\\([^\\\"]*\\\)\\\".*/\\1/p\" $ROOTFS$icd'" | head -1)
        if [ -n "$drv" ] && adb_ "run-as $PKG sh -c 'test -r $ROOTFS$drv && echo yes'" | grep -q yes; then
            ok "manifest library_path resolves in the tree ($drv)"
        else
            bad "manifest library_path does not resolve in the tree (got: ${drv:-<none>})"
        fi
    fi
fi

echo "== push probe =="
if [ ! -x "$GL_PROBE" ]; then
    bad "missing $GL_PROBE (run tools/guest-config/build-guest-tools.sh)"
else
    push "$HERE/husk-exec.sh" tmp/husk-exec.sh &&
        push "$GL_PROBE" root/gl-probe &&
        adb_ "run-as $PKG sh -c 'chmod 755 $ROOTFS/root/gl-probe'" >/dev/null &&
        ok "probe pushed"
fi

echo "== loader defaults, no GPU env of any kind =="
r=$(renderer plain)
echo "  STATE plain renderer: $r"
case "$r" in
    *zink*Mali-G77*) ok "a plain exec inherits hardware GL from the loader alone" ;;
    *llvmpipe*)      bad "plain exec fell back to llvmpipe: loader defaults are not reaching non-session processes" ;;
    *)               bad "unexpected renderer: ${r:-<empty>}" ;;
esac

echo "== negative control: NR_GPU_DEFAULT_OFF=1 =="
r=$(renderer off NR_GPU_DEFAULT_OFF=1)
echo "  STATE off renderer: $r"
case "$r" in
    *llvmpipe*) ok "NR_GPU_DEFAULT_OFF=1 really turns the defaults off" ;;
    *)          bad "NR_GPU_DEFAULT_OFF=1 did not turn them off (got: ${r:-<empty>})" ;;
esac

echo "== negative control: HUSK_GPU=llvmpipe =="
r=$(renderer soft HUSK_GPU=llvmpipe)
echo "  STATE soft renderer: $r"
case "$r" in
    *llvmpipe*) ok "HUSK_GPU=llvmpipe wins over the file" ;;
    *)          bad "HUSK_GPU=llvmpipe did not win (got: ${r:-<empty>})" ;;
esac

echo "GPU_LOADER_DEFAULTS PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
[ "$FAIL" -eq 0 ] || exit 1
[ "$have_driver" = true ] || exit 2
exit 0
