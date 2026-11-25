#!/bin/sh
# husk-desktop.sh - 起一个可用的 X11 桌面会话（单一入口）。
#
# 为什么需要它（evidence 194/200，2026-11-25 实测）：
#   在那之前，桌面的启动流程只存在于开发机的临时脚本里，没有人能复现，于是每次
#   "手起"会话都会留下不同的残骸：
#     * 上一个 shell 被 SIGKILL 掉 -> gnome-shell 认为上次崩过，下一次进安全模式
#       （XDG_RUNTIME_DIR/gnome-shell-disable-extensions 在场时扩展全不加载）；
#     * XDG_RUNTIME_DIR 里的 gvfsd / at-spi / dbus socket 是上一次的，绑定失败
#       -> "peer-to-peer connection failed: Address already in use"；
#     * 重复的 colord / accounts-daemon / upowerd 攒一堆；
#     * 壁纸的预缩放看护进程没被调用，桌面一片底色。
#   这些都表现为"0.0.6 预备不稳定"，但根子是"没有单一入口 + 启动不自检"。
#
# usage: husk-desktop.sh <rootfs-dir> [--home DIR] [--wait SEC]
#
# 环境变量：
#   NR_LD      加载器（默认 <rootfs 的父目录>/libld.so）
#   DISPLAY    X 显示（默认 :0；X server 由应用进程持有，本脚本不重启它）
#
# 谁在这个会话里：gnome-shell 自己就是 WM + compositor（mutter 46 X11 后端），
# DING 扩展负责桌面图标和壁纸；gsd-* / 门户 / gvfs 由 shell 按需拉起。
#
# 为什么不用假的 uid：试过 NR_FAKE_UID=1000（让 shell 不弹"privileged user"），
# 但 dbus 的 EXTERNAL 认证会比对真实 uid，客户端声明 1000、服务器看到 2xxxx，
# 直接被拒："Failed to connect to session bus: The connection is closed"。
# 所以会话仍以 root 身份跑，那条提示按"已知提示"处理，不用假身份绕。
set -eu

BASE=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 1 ] || { echo "usage: husk-desktop.sh <rootfs> [--home DIR] [--wait SEC]" >&2; exit 64; }
ROOTFS=$(cd "$1" && pwd); shift
HOME_G=/root
WAIT=30
while [ $# -gt 0 ]; do
    case "$1" in
        --home) [ $# -ge 2 ] || exit 64; HOME_G=$2; shift 2 ;;
        --wait) [ $# -ge 2 ] || exit 64; WAIT=$2; shift 2 ;;
        *) echo "husk-desktop: unknown option $1" >&2; exit 64 ;;
    esac
done

NR_LD=${NR_LD:-$(dirname "$ROOTFS")/libld.so}
[ -x "$NR_LD" ] || { echo "husk-desktop: loader not executable: $NR_LD" >&2; exit 69; }
DISPLAY=${DISPLAY:-:0}
RT=${XDG_RUNTIME_DIR:-$ROOTFS/tmp/xdg-husk}
SYSBUS=$ROOTFS/run/dbus/system_bus_socket
SESSBUS=$RT/bus
STATE=$(dirname "$ROOTFS")/.husk-install-state
LIBS="$ROOTFS/lib/aarch64-linux-gnu:$ROOTFS/usr/lib/aarch64-linux-gnu"
[ -d "$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14" ] && LIBS="$LIBS:$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14"
[ -d "$ROOTFS/usr/lib/gnome-shell" ] && LIBS="$LIBS:$ROOTFS/usr/lib/gnome-shell"
if [ -f "$ROOTFS/etc/ld.so.husk-path" ]; then
    LIBS="$LIBS:$(sed -n 's|^|'"$ROOTFS"'|p' "$ROOTFS/etc/ld.so.husk-path" | tr '\n' ':')"
fi

# 在 guest 里跑一条命令（不继承宿主的 XDG_*，避免把宿主路径漏进去）
g() {
    ( cd "$ROOTFS" && env -u XDG_CACHE_HOME -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME \
        HOME="$HOME_G" PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_LIBRARY_PATH="$LIBS" \
        DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS" \
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SYSBUS" \
        "$NR_LD" --library-path "$LIBS" "$@" ) 2>/dev/null
}
# 同上，但要后台常驻、日志进文件
g_bg() {
    logf=$1; shift
    ( cd "$ROOTFS" && nohup env -u XDG_CACHE_HOME -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME \
        HOME="$HOME_G" PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_LIBRARY_PATH="$LIBS" \
        DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS" \
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SYSBUS" \
        "$NR_LD" --library-path "$LIBS" "$@" > "$logf" 2>&1 & )
}

say() { echo "husk-desktop: $*" >&2; }

# ---------------------------------------------------------------- 1. 停残留
# 只按 guest 程序名匹配，绝不碰应用自己的进程（X server 在应用进程里）。
stop_leftovers() {
    for pat in usr/bin/gnome-shell mutter-x11-frames usr/bin/gjs gnome-settings-daemon \
               /usr/libexec/gsd- xdg-desktop-portal gvfsd tracker-miner at-spi \
               usr/bin/nautilus usr/bin/gnome-text-editor usr/bin/gnome-calculator; do
        g usr/bin/pkill -f "$pat" || true
    done
    sleep 2
    for pat in usr/bin/gnome-shell mutter-x11-frames usr/bin/gjs gnome-settings-daemon \
               /usr/libexec/gsd- xdg-desktop-portal gvfsd tracker-miner at-spi; do
        g usr/bin/pkill -9 -f "$pat" || true
    done
    # 总线自己最后停，避免上面那些进程在收尾时又把它拉起来
    g usr/bin/pkill -f "dbus-daemon --session" || true
    g usr/bin/pkill -f "dbus-daemon --config-file" || true
    sleep 2
}
stop_leftovers

# ------------------------------------------------------------ 2. 清陈旧状态
# gnome-shell-disable-extensions 是安全模式标记：它一在场，扩展（含 DING、dock、
# user-theme）就不加载，表现就是"主题全丢了"。
rm -f "$RT/bus" "$RT/bus.lock" "$RT/gnome-shell-disable-extensions" 2>/dev/null || true
rm -f "$SYSBUS" "$ROOTFS/run/dbus/pid" 2>/dev/null || true
rm -rf "$RT/gvfsd" "$RT/gvfs" "$RT/at-spi" 2>/dev/null || true
mkdir -p "$RT" "$ROOTFS/run/dbus" 2>/dev/null || true
chmod 700 "$RT" 2>/dev/null || true

# ------------------------------------------------------------------ 3. 总线
g_bg "$ROOTFS/tmp/husk-sysbus.log" usr/bin/dbus-daemon \
     --config-file=/usr/local/share/linuxhub/linuxhub-system-bus.conf \
     --nofork --address=unix:path=/run/dbus/system_bus_socket
i=0; while [ $i -lt 20 ] && [ ! -S "$SYSBUS" ]; do sleep 1; i=$((i+1)); done
[ -S "$SYSBUS" ] || say "WARNING: 系统总线没起来（账号/挂载相关功能会退化）"

g_bg "$ROOTFS/tmp/husk-sessbus.log" usr/bin/dbus-daemon \
     --session --nofork --address="unix:path=$SESSBUS"
i=0; while [ $i -lt 20 ] && [ ! -S "$SESSBUS" ]; do sleep 1; i=$((i+1)); done
[ -S "$SESSBUS" ] || { say "会话总线起不来，会话无法继续"; exit 70; }

# ------------------------------------------------------------- 4. gnome-shell
SHELL_LOG=$ROOTFS/tmp/husk-shell.log
: > "$SHELL_LOG" 2>/dev/null || true
(
    cd "$ROOTFS" || exit 1
    nohup env -u XDG_CACHE_HOME -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME \
        HOME="$HOME_G" PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_LIBRARY_PATH="$LIBS" \
        DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS" \
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SYSBUS" \
        XDG_CURRENT_DESKTOP=ubuntu:GNOME XDG_SESSION_DESKTOP=ubuntu \
        XDG_SESSION_TYPE=x11 DESKTOP_SESSION=ubuntu \
        GNOME_SHELL_SESSION_MODE=linuxhub-ubuntu-min \
        XDG_DATA_DIRS=/usr/local/share:/usr/share XDG_CONFIG_DIRS=/etc/xdg \
        GSETTINGS_SCHEMA_DIR=/usr/share/glib-2.0/schemas \
        GI_TYPELIB_PATH=/usr/lib/aarch64-linux-gnu/girepository-1.0:/usr/lib/girepository-1.0 \
        GJS_PATH=/usr/share/gnome-shell \
        XCURSOR_THEME=Yaru XCURSOR_SIZE=24 XKB_CONFIG_ROOT="$ROOTFS/usr/share/X11/xkb" \
        GDK_BACKEND=x11 CLUTTER_BACKEND=x11 MUTTER_DEBUG_FORCE_KMS_MODE=0 MESA_NO_ERROR=1 \
        COGL_DRIVER=gles2 MESA_GLES_VERSION_OVERRIDE=3.2 GJS_DISABLE_JIT=1 \
        "$NR_LD" --library-path "$LIBS" usr/bin/gnome-shell --x11 --replace > "$SHELL_LOG" 2>&1 &
) || true

# 等 WM 出现（不是"进程在"，而是"有人认领 _NET_SUPPORTING_WM_CHECK"）
i=0
while [ $i -lt "$WAIT" ]; do
    if g usr/bin/xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null | grep -q 'window id'; then break; fi
    sleep 1; i=$((i+1))
done

# ---------------------------------------------------------------- 5. 自检
wm=no
g usr/bin/xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null | grep -q 'window id' && wm=yes
ding=no
g usr/bin/xwininfo -root -children 2>/dev/null | grep -q 'Desktop Icons' && ding=yes
gpu=unknown
if g usr/bin/gl-probe --query 2>/dev/null | grep -qi zink; then gpu=zink
elif g usr/bin/gl-probe --query 2>/dev/null | grep -qi llvmpipe; then gpu=llvmpipe
fi

echo "CHECK wm=$wm"
echo "CHECK ding=$ding"
echo "CHECK gpu=$gpu"
echo "CHECK shell_log=$SHELL_LOG"

# 壁纸：装好的看护脚本会把当前壁纸缩到屏幕尺寸（并修复空/失效的 picture-uri）
if [ -x "$BASE/husk-wallpaper-fit.sh" ]; then
    DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT" DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS" \
        "$BASE/husk-wallpaper-fit.sh" "$ROOTFS" "$NR_LD" "$HOME_G" >/dev/null 2>&1 &
fi

[ "$wm" = yes ] || { say "会话没起来：$SHELL_LOG"; exit 71; }
say "会话已起（wm=$wm ding=$ding gpu=$gpu）"
