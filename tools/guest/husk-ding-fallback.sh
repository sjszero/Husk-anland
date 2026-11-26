#!/bin/sh
# husk-ding-fallback.sh - 兜住"桌面图标 + 壁纸"。
#
# 为什需要：gnome-shell 自己那条 DING 启动链在这个环境里传的 `-P` 是空串，
# ding.js 于是把 stylesheet 解析成 `/stylesheet.css`，加载 CSS 时抛异常退出；
# DING 一死，桌面既没有图标也没有壁纸（壁纸是 DING 画的），只剩 primary-color 纯色底。
# 实测见 docs/design/evidence/191-session-restart-ding-20261125.md §4/§7。
#
# 做法：等 shell 把 X 拉起来 → 看有没有 "Desktop Icons" 窗口 → 等不到就自己按
# 正确参数起一个 DING。启动参数与手工验证过的那条完全一致。
#
# usage: husk-ding-fallback.sh <rootfs> <loader>
#        环境：NR_ROOTFS/NR_LD 由调用方给，其余从 guest 约定里补。
set -u

ROOTFS=${1:?rootfs}
NR_LD=${2:?loader}
# The whole script `cd`s into the rootfs before calling the loader, so a relative
# loader path silently breaks every probe (each call just fails, hidden by
# 2>/dev/null) and the watcher ends up waiting the full 90 s and then launching
# nothing. Fail fast instead -- this cost one debugging session already.
case "$NR_LD" in
    /*) ;;
    *)  echo "husk-ding-fallback: loader must be an absolute path (got: $NR_LD)" >&2
        exit 2 ;;
esac
LIBS="$ROOTFS/lib/aarch64-linux-gnu:$ROOTFS/usr/lib/aarch64-linux-gnu"
[ -d "$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14" ] && LIBS="$LIBS:$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14"
[ -d "$ROOTFS/usr/lib/gnome-shell" ] && LIBS="$LIBS:$ROOTFS/usr/lib/gnome-shell"
[ -d "$ROOTFS/usr/lib/aarch64-linux-gnu/gvfs" ] && LIBS="$LIBS:$ROOTFS/usr/lib/aarch64-linux-gnu/gvfs"
if [ -f "$ROOTFS/etc/ld.so.husk-path" ]; then
    _runpaths=$(sed -n '/^#/d; s|^|'"$ROOTFS"'|p' "$ROOTFS/etc/ld.so.husk-path" | tr '\n' ':')
    LIBS="$LIBS:$_runpaths"
fi

DING_DIR=/usr/share/gnome-shell/extensions/ding@rastersoft.com/app

g() {
    ( cd "$ROOTFS" && "$NR_LD" --library-path "$LIBS" usr/bin/env -i \
        HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_SESSION_ID="${NR_SESSION_ID:-}" DISPLAY=:0 \
        "$@" ) 2>/dev/null
}

# 等 X 起来（最多 60 s）
i=0
while [ $i -lt 60 ]; do
    if g usr/bin/husk-window-wait --geometry 2>/dev/null | grep -Eq '^[0-9]+x[0-9]+$'; then break; fi
    sleep 1; i=$((i+1))
done

# 等 shell 自己把 DING 拉起来（最多 30 s）
if g usr/bin/husk-window-wait --wait-window "Desktop Icons" 30000 2>/dev/null | grep -q '^window=yes$'; then
    exit 0
fi

# 还是没起来：自己拉一个。X11 探测统一走随树安装的 Rust helper，不再依赖
# x11-utils/xdotool；精简桌面树没有这几个测试包时，看护进程也不能静默失效。
GEO=$(g usr/bin/husk-window-wait --geometry 2>/dev/null | head -1)
case "$GEO" in
    [0-9]*x[0-9]*) ;;
    *) GEO=2400x1080 ;;
esac
W=${GEO%x*}; H=${GEO#*x}

( cd "$ROOTFS" && "$NR_LD" --library-path "$LIBS" usr/bin/env -i \
    HOME=/root USER=root LOGNAME=root \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_SESSION_ID="${NR_SESSION_ID:-}" DISPLAY=:0 GDK_BACKEND=x11 \
    XDG_DATA_DIRS=/usr/local/share:/usr/share XDG_CONFIG_DIRS=/etc/xdg \
    XDG_CURRENT_DESKTOP=ubuntu:GNOME XDG_SESSION_DESKTOP=ubuntu XDG_SESSION_TYPE=x11 \
    GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas \
    GI_TYPELIB_PATH=/usr/lib/aarch64-linux-gnu/girepository-1.0:/usr/lib/girepository-1.0 \
    GJS_PATH="$DING_DIR:/usr/share/gnome-shell" \
    /usr/bin/gjs "$DING_DIR/ding.js" -P "$DING_DIR" \
    -D "0:0:$W:$H:1:0:0:0:0:0" -M 0 \
    >> "$ROOTFS/tmp/husk-ding.log" 2>&1 & )
echo "husk-ding-fallback: launched DING ${W}x${H}" >> "$ROOTFS/tmp/husk-ding.log"
exit 0
