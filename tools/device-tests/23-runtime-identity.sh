#!/bin/sh
# 23 · 运行身份自证：设备上这份树和 loader 是不是"我们要测的那份"。
#
# 没有这一条，任何真机数字都只能算"某天某棵树上的现场"。安装器在
# <rootfs>/etc/husk/runtime-identity 里写下六项身份（version / commit / loader /
# libc / gpu-env / driver / icd），这里**重新算一遍**再比对：文件被热替换过、
# 树被别的流程动过、或者部署的还是旧包，都会在这里现形。
#
#   DEV=<adb serial> PKG=<the host package that owns the tree> \
#   ROOTFS=files/linux/distributions/ubuntu-24.04/rootfs \
#   LD=files/linux/husk/libld.so sh 23-runtime-identity.sh
#
# DIST_IDENTITY 指向本次构建产出的 dist/RUNTIME-IDENTITY（同仓库里的构建产物）；
# 有它时，设备上的 commit/version 必须与构建一致，否则判 FAIL。
set -u
DEV=${DEV:?usage: DEV=<adb serial> [PKG=...] [ROOTFS=...] [LD=...] $0}
PKG=${PKG:-com.linuxhub.myth.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
DIST_IDENTITY=${DIST_IDENTITY:-$HERE/../../dist/RUNTIME-IDENTITY}
REPO_VERSION_FILE=${REPO_VERSION_FILE:-$HERE/../../VERSION}
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }

# sha of a path relative to the app's data dir (the dir run-as starts in).
sha() {
    adb -s "$DEV" shell "run-as $PKG sh -c 'sha256sum $1 2>/dev/null'" 2>/dev/null |
        awk 'NF{print $1; exit}'
}
# value of a key in the guest's runtime-identity file.
id_field() {
    adb -s "$DEV" shell "run-as $PKG sh -c 'cat $ROOTFS/etc/husk/runtime-identity 2>/dev/null'" 2>/dev/null |
        sed -n "s/^$1=//p" | head -1 | tr -d '\r'
}

echo "== runtime identity record =="
ident=$(adb -s "$DEV" shell "run-as $PKG sh -c 'cat $ROOTFS/etc/husk/runtime-identity 2>/dev/null'" 2>/dev/null | tr -d '\r')
if [ -n "$ident" ]; then
    printf '%s\n' "$ident" | sed 's/^/    /'
    ok "identity file present ($(printf '%s\n' "$ident" | wc -l | tr -d ' ') fields)"
else
    bad "$ROOTFS/etc/husk/runtime-identity missing: this deployment cannot prove what it is"
fi

v_version=$(id_field version)
v_commit=$(id_field git_commit)
v_mode=$(id_field mode)
echo "  STATE version=${v_version:-?} commit=${v_commit:-?} mode=${v_mode:-?}"

echo "== recomputed hashes vs the record =="
# Every entry: <identity key> <path used to recompute> <what it is>.
check() {
    key=$1; path=$2; label=$3
    want=$(id_field "$key")
    have=$(sha "$path")
    if [ -z "$want" ] || [ "$want" = missing ]; then
        bad "$key not recorded"
    elif [ -z "$have" ]; then
        bad "$key: $label unreadable at $path"
    elif [ "$want" = "$have" ]; then
        ok "$key ${have%"${have#????????????}"}… $label"
    else
        bad "$key drifted: record=${want%"${want#????????????}"}… disk=${have%"${have#????????????}"}… ($label)"
    fi
}
check loader_sha256 "$LD" "loader in the app dir"
# libc: the installer restores the bundled one into usr/lib/aarch64-linux-gnu and
# only falls back to lib/ when that name does not exist there.
libc_path="$ROOTFS/usr/lib/aarch64-linux-gnu/libc.so.6"
if [ -z "$(sha "$libc_path")" ]; then libc_path="$ROOTFS/lib/aarch64-linux-gnu/libc.so.6"; fi
check libc_sha256 "$libc_path" "guest libc"
check gpu_env_sha256 "$ROOTFS/etc/husk/gpu-env" "gpu-env"
check panvk_driver_sha256 "$ROOTFS/usr/lib/aarch64-linux-gnu/libvulkan_panfrost_husk.so" "PanVK driver"
check icd_sha256 "$ROOTFS/usr/share/vulkan/icd.d/husk_panvk_guest.json" "canonical ICD"
check pipewire_plugin_sha256 "$ROOTFS/usr/lib/aarch64-linux-gnu/gstreamer-1.0/libgstpipewire.so" "gstpipewiresrc fix"

echo "== this build vs what is deployed =="
if [ -f "$REPO_VERSION_FILE" ]; then
    repo_version=$(tr -d ' \r\n' < "$REPO_VERSION_FILE")
    if [ "$v_version" = "$repo_version" ]; then
        ok "VERSION matches the checkout ($repo_version)"
    else
        bad "VERSION mismatch: deployed=${v_version:-?} checkout=$repo_version"
    fi
fi
if [ -f "$DIST_IDENTITY" ]; then
    b_commit=$(sed -n 's/^commit=//p' "$DIST_IDENTITY" | head -1)
    b_loader=$(sed -n 's/^loader_sha256=//p' "$DIST_IDENTITY" | head -1)
    b_version=$(sed -n 's/^version=//p' "$DIST_IDENTITY" | head -1)
    [ "$v_commit" = "$b_commit" ] \
        && ok "commit matches dist/RUNTIME-IDENTITY ($b_commit)" \
        || bad "commit mismatch: deployed=${v_commit:-?} dist=${b_commit:-?}"
    [ "$v_version" = "$b_version" ] \
        && ok "dist build version matches ($b_version)" \
        || bad "version mismatch: deployed=${v_version:-?} dist=${b_version:-?}"
    want_loader=$(id_field loader_sha256)
    [ "$want_loader" = "$b_loader" ] \
        && ok "loader is the one this dist shipped" \
        || bad "loader is not the dist one: deployed=${want_loader%"${want_loader#????????????}"}… dist=${b_loader%"${b_loader#????????????}"}…"
else
    echo "  SKIP  no $DIST_IDENTITY"
fi

echo "== no stray old artifacts =="
old=$(adb -s "$DEV" shell "run-as $PKG sh -c 'ls $ROOTFS/usr/lib/aarch64-linux-gnu/ | grep -E \"^libvulkan_panfrost_(frist|guest|old).*[.]so\$\"'" 2>/dev/null | tr -d '\r')
if [ -n "$old" ]; then
    bad "stale driver copies in the tree: $(printf '%s' "$old" | tr '\n' ' ')"
else
    ok "no stale PanVK driver copies"
fi

echo "RUNTIME_IDENTITY PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
