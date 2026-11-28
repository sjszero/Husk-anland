#!/bin/sh
# 26 · 用本次构建的发行包，在真机宿主 App 里做一次 --upgrade 部署。
#
# 为什么是升级而不是重装：整棵树 4.3 GB，重解一遍要几十分钟；--upgrade 只重做
# guest 侧步骤（loader/工具/ICD/驱动/配置/locale/符号链接/身份），不动 /home 和
# dpkg 状态。安装器自己会先停掉指向这棵树的 guest 进程——热替换正在映射的
# libvulkan_panfrost_husk.so 会把进程打死在缺页上（SIGBUS 135，2026-11-27 实测）。
#
#   DEV=<adb serial> PKG=<the host package that owns the tree> \
#   ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs \
#   LD=files/linux/husk/libld.so sh 26-upgrade-in-app.sh
#
# 装完立刻跑 23-runtime-identity.sh：身份六项重算必须与记录一致，否则这次部署
# 不能用来出任何数字。
set -eu
DEV=${DEV:?usage: DEV=<adb serial> [PKG=...] [ROOTFS=...] [TARBALL=...] $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
TARBALL=${TARBALL:-$HERE/../../dist/husk-arm64.tar.gz}
STAGE=${STAGE:-files/linux/husk-stage}

[ -f "$TARBALL" ] || { echo "no tarball: $TARBALL (run make dist)" >&2; exit 66; }
echo "== tarball =="
echo "   $TARBALL  $(sha256sum "$TARBALL" | cut -d' ' -f1)"
adb -s "$DEV" push "$TARBALL" /data/local/tmp/husk-arm64.tar.gz >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'rm -rf $STAGE && mkdir -p $STAGE && cp /data/local/tmp/husk-arm64.tar.gz $STAGE/p.tar.gz && tar xzf $STAGE/p.tar.gz -C $STAGE && rm -f $STAGE/p.tar.gz && ls $STAGE | head -5'" | sed 's/^/   /'
adb -s "$DEV" shell "rm -f /data/local/tmp/husk-arm64.tar.gz"

echo "== install --upgrade =="
out=$(adb -s "$DEV" shell "run-as $PKG sh $STAGE/install-husk.sh --upgrade --dest $ROOTFS --loader $STAGE/libld.so" 2>&1)
rc=$?
printf '%s\n' "$out" | tail -40 | sed 's/^/   /'
# 安装器只在自己判定成功时才打印 "== done =="；exit code 经由 adb 一路回来不可靠
# （adb shell 会吞掉部分信号退出码），所以两个条件都要求。
case "$out" in
    *"== done =="*) ;;
    *) echo "install-husk did not reach its done marker (rc=$rc)" >&2; exit 70 ;;
esac
echo "   PASS  installer reached its done marker (rc=$rc)"

echo "== installed runtime vs this build =="
BASE=${ROOTFS%/*}
CMP_PASS=0
CMP_FAIL=0
sha_at() {
    adb -s "$DEV" shell "run-as $PKG sh -c 'sha256sum $1 2>/dev/null'" 2>/dev/null |
        awk 'NF{print $1;exit}'
}
compare_file() {
    label=$1
    installed=$2
    staged=$3
    a=$(sha_at "$installed")
    b=$(sha_at "$STAGE/$staged")
    if [ -n "$a" ] && [ "$a" = "$b" ]; then
        CMP_PASS=$((CMP_PASS+1))
        echo "  PASS  $label identical to this build"
    else
        CMP_FAIL=$((CMP_FAIL+1))
        echo "  FAIL  $label differs: installed=${a:-missing} stage=${b:-missing}"
    fi
}
compare_file 'app libld.so' 'files/linux/husk/libld.so' 'libld.so'
compare_file 'app husk-logd' 'files/linux/husk/husk-logd' 'husk-logd'
compare_file 'guest usr/sbin/husk-usermgr' "$ROOTFS/usr/sbin/husk-usermgr" 'husk-usermgr'
compare_file 'guest usr/bin/husk-usermgr' "$ROOTFS/usr/bin/husk-usermgr" 'husk-usermgr'
compare_file 'guest usr/sbin/husk-su' "$ROOTFS/usr/sbin/husk-su" 'husk-su'
compare_file 'guest usr/bin/husk-su' "$ROOTFS/usr/bin/husk-su" 'husk-su'
compare_file 'guest usr/sbin/husk-ldconfig' "$ROOTFS/usr/sbin/husk-ldconfig" 'husk-ldconfig'
compare_file 'guest usr/bin/husk-ldconfig' "$ROOTFS/usr/bin/husk-ldconfig" 'husk-ldconfig'
compare_file 'guest husk-symlinks' "$ROOTFS/usr/bin/husk-symlinks" 'husk-symlinks'
compare_file 'guest husk-window-wait' "$ROOTFS/usr/bin/husk-window-wait" 'husk-window-wait'
compare_file 'guest husk-logd' "$ROOTFS/usr/bin/husk-logd" 'husk-logd'
compare_file 'guest husk-media-session' "$ROOTFS/usr/local/bin/husk-media-session" 'husk-media-session.sh'
compare_file 'guest wallpaper-scale' "$ROOTFS/usr/local/share/linuxhub/wallpaper-scale" 'guest/wallpaper-scale'
compare_file 'session husk-desktop' "$BASE/husk-desktop.sh" 'husk-desktop.sh'
compare_file 'session husk-wallpaper-fit' "$BASE/husk-wallpaper-fit.sh" 'husk-wallpaper-fit.sh'
compare_file 'session husk-ding-fallback' "$BASE/husk-ding-fallback.sh" 'husk-ding-fallback.sh'
compare_file 'guest audio-setup' "$ROOTFS/usr/bin/audio-setup" 'audio-setup'
compare_file 'guest audio-relay' "$ROOTFS/usr/bin/audio-relay" 'audio-relay'
compare_file 'guest audio-capture' "$ROOTFS/usr/bin/audio-capture" 'audio-capture'
compare_file 'guest audio-probe' "$ROOTFS/usr/bin/audio-probe" 'audio-probe'
echo "  RUNTIME_COMPARE PASS=$CMP_PASS FAIL=$CMP_FAIL"
[ "$CMP_FAIL" -eq 0 ] || exit 71

echo "== self-proof =="
DEV="$DEV" PKG="$PKG" ROOTFS="$ROOTFS" LD="$LD" sh "$HERE/23-runtime-identity.sh"
