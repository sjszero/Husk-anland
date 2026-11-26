#!/bin/sh
# husk-wallpaper-fit.sh - 会话起来后把壁纸预先缩到屏幕尺寸。
#
# 为什么需要（evidence 195，2026-11-25 实测）：
#   rootfs 里自带的 Ubuntu 壁纸是 3840x2160，屏幕是 2400x1080，mutter 必须缩小采样。
#   在 zink/PanVK 这条路上，"缩小采样的壁纸"会被画错：屏幕右侧一块三角里整幅壁纸
#   只有 32% 亮度（内容、几何都对，就是被乘了 0.32；用白底网格图和 0..255 渐变图
#   各验过一次）。同一个会话里把壁纸换成已经等于屏幕尺寸的同一张图，逐像素比对
#   1.00 吻合、楔形消失。llvmpipe 会话是另一种残缺（只画 0..479 行），与壁纸尺寸无关。
#
# 做法：等 shell 把 X 和会话总线拉起来，读 xrandr 的 current 尺寸，把当前壁纸缩放成
# 恰好屏幕大小的 PNG 放进 ~/.cache/husk/，然后把 picture-uri / picture-uri-dark
# 指过去。纯色底（picture-uri 不是 file://）不动；失败一律静默退出 —— 壁纸是外观
# 问题，不能因为它挡住宿主会话。
#
# usage: husk-wallpaper-fit.sh <rootfs> <loader> [HOME]
set -u

ROOTFS=${1:?rootfs}
NR_LD=${2:?loader}
HOME_G=${3:-/root}
# cd 进 rootfs 之后相对的 loader 路径会静默失败（ding-fallback 踩过一次），先挡住。
case "$NR_LD" in
    /*) ;;
    *)  echo "husk-wallpaper-fit: loader must be an absolute path (got: $NR_LD)" >&2
        exit 2 ;;
esac
LIBS="$ROOTFS/lib/aarch64-linux-gnu:$ROOTFS/usr/lib/aarch64-linux-gnu"
[ -d "$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14" ] && LIBS="$LIBS:$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14"
[ -d "$ROOTFS/usr/lib/gnome-shell" ] && LIBS="$LIBS:$ROOTFS/usr/lib/gnome-shell"

SCALE_JS=/usr/local/share/linuxhub/husk-wallpaper-scale.js
SCHEMA=org.gnome.desktop.background

g() {
    ( cd "$ROOTFS" && "$NR_LD" --library-path "$LIBS" usr/bin/env -i \
        HOME="$HOME_G" PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_SESSION_ID="${NR_SESSION_ID:-}" DISPLAY="${DISPLAY:-:0}" \
        DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-}" \
        XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/runtime-husk}" \
        XDG_DATA_DIRS=/usr/local/share:/usr/share \
        "$@" ) 2>/dev/null
}

# 1. 等 shell 把 X 拉起来、会话总线能应答（最多 90 s）
i=0
while [ $i -lt 90 ]; do
    if g usr/bin/husk-window-wait --geometry >/dev/null 2>&1; then break; fi
    sleep 1; i=$((i+1))
done
[ $i -lt 90 ] || exit 0

# 2. 屏幕尺寸（Rust helper 只 dlopen libX11，不要求 x11-utils）
GEO=$(g usr/bin/husk-window-wait --geometry 2>/dev/null | head -1)
case "$GEO" in
    [0-9]*x[0-9]*) W=${GEO%x*}; H=${GEO#*x} ;;
    *) exit 0 ;;
esac

# 3. 当前壁纸。空 URI、不是 file://、或者指向的文件不在，都算"壁纸坏了"，兜底到一张
#    随发行版装进来的图（2026-11-25 实测发现 picture-uri 被留成空串、primary-color 被
#    留成 #00ff00 时，桌面就是一片黑/荧光绿，用户看到的是"桌面背景老是变样"）。
URI=$(g usr/bin/gsettings get $SCHEMA picture-uri | tr -d "'" | head -1)
need_default=0
case "${URI:-}" in
    file://*) [ -f "$ROOTFS${URI#file://}" ] || need_default=1 ;;
    *) need_default=1 ;;
esac
if [ "$need_default" = 1 ]; then
    DEF=
    for cand in usr/share/backgrounds/husk-default.png \
                usr/share/backgrounds/Clouds_by_Tibor_Mokanszki.jpg \
                usr/share/backgrounds/*.png \
                usr/share/backgrounds/*.jpg; do
        [ -f "$ROOTFS/$cand" ] && { DEF=/$cand; break; }
    done
    [ -n "$DEF" ] || exit 0
    g usr/bin/gsettings set $SCHEMA picture-uri "file://$DEF" || exit 0
    g usr/bin/gsettings set $SCHEMA picture-uri-dark "file://$DEF" >/dev/null 2>&1 || true
    g usr/bin/gsettings set $SCHEMA primary-color '#023c88' >/dev/null 2>&1 || true
    URI="file://$DEF"
    echo "husk-wallpaper-fit: 壁纸缺失或失效，兜底 -> $DEF" >&2
fi
SRC=${URI#file://}

CACHE="$HOME_G/.cache/husk"
OUT="$CACHE/wallpaper-${W}x${H}.png"

# 4. 缓存比源图新就直接复用，否则重算
g usr/bin/mkdir -p "$CACHE" || exit 0
if [ -s "$ROOTFS$OUT" ] && [ "$ROOTFS$OUT" -nt "$ROOTFS$SRC" ]; then
    :
else
    g usr/bin/gjs "$SCALE_JS" "$SRC" "$OUT" "$W" "$H"
    # 3 = 源图本来就等于屏幕尺寸：没有可写的，也就不要改 picture-uri
    [ $? -eq 0 ] || exit 0
fi

[ -s "$ROOTFS$OUT" ] || exit 0
g usr/bin/gsettings set $SCHEMA picture-uri "file://$OUT" || exit 0
g usr/bin/gsettings set $SCHEMA picture-uri-dark "file://$OUT" >/dev/null 2>&1 || true
g usr/bin/gsettings set $SCHEMA picture-options zoom >/dev/null 2>&1 || true
echo "husk-wallpaper-fit: picture-uri -> $OUT (${W}x${H})" >&2
