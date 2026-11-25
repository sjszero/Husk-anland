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

# Android 给应用的 umask 是 0000（`run-as ... umask` 打印 000），会话继承它之后
# 每个新目录都是 0777。DING 拒绝从世界可写的 ~/Desktop 启动桌面文件，所以这里
# 和 husk-session.sh 一样，在入口处统一设成普通登录会话的 022。
umask 022

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
# 同一个路径有两个身份：宿主看到的（$RT）和 guest 看到的（$RT_G）。dbus 的地址、
# XDG_RUNTIME_DIR 必须用 guest 那一份 —— 用宿主绝对路径写进 unix:path= 时，
# 路径层会再加一次 rootfs 前缀，于是连不上（实测报 "Could not connect:
# No such file or directory"，main.js 直接抛异常退出）。
case "$RT" in
    "$ROOTFS"/*) RT_G=/${RT#"$ROOTFS"/} ;;
    *)           RT_G=$RT ;;
esac
SYSBUS=$ROOTFS/run/dbus/system_bus_socket
SYSBUS_G=/run/dbus/system_bus_socket
SESSBUS=$RT/bus
SESSBUS_G=$RT_G/bus
STATE=$(dirname "$ROOTFS")/.husk-install-state
LIBS="$ROOTFS/lib/aarch64-linux-gnu:$ROOTFS/usr/lib/aarch64-linux-gnu"
[ -d "$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14" ] && LIBS="$LIBS:$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14"
[ -d "$ROOTFS/usr/lib/gnome-shell" ] && LIBS="$LIBS:$ROOTFS/usr/lib/gnome-shell"
if [ -f "$ROOTFS/etc/ld.so.husk-path" ]; then
    LIBS="$LIBS:$(sed -n 's|^|'"$ROOTFS"'|p' "$ROOTFS/etc/ld.so.husk-path" | tr '\n' ':')"
fi

# 桌面 UI 的语言由 LANG 决定，而这个变量经常是空的（run-as 起的进程、env -i 都不带）。
# 树里有 zh_CN.utf8 就用中文；调用者自己设了 LANG 就不动。
case "${LANG:-}" in
    ''|POSIX|C|C.UTF-8)
        if [ -f "$ROOTFS/usr/lib/locale/locale-archive" ] || [ -d "$ROOTFS/usr/lib/locale/zh_CN.utf8" ]; then
            G_LANG=zh_CN.UTF-8
        else
            G_LANG=C.UTF-8
        fi ;;
    *) G_LANG=$LANG ;;
esac
: "${LANGUAGE:=zh_CN:zh}"
G_LANGUAGE=$LANGUAGE

# 在 guest 里跑一条命令（不继承宿主的 XDG_*，避免把宿主路径漏进去）
g() {
    ( cd "$ROOTFS" && env -u XDG_CACHE_HOME -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME \
        HOME="$HOME_G" PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_LIBRARY_PATH="$LIBS" \
        DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT_G" \
        LANG="$G_LANG" LANGUAGE="$G_LANGUAGE" \
        GTK_IM_MODULE=fcitx QT_IM_MODULE=fcitx XMODIFIERS=@im=fcitx \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS_G" \
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SYSBUS_G" \
        "$NR_LD" --library-path "$LIBS" "$@" ) 2>/dev/null
}
# 同上，但要后台常驻、日志进文件
g_bg() {
    logf=$1; shift
    ( cd "$ROOTFS" || exit 1
      exec 0</dev/null 1>/dev/null 2>/dev/null
      nohup env -u XDG_CACHE_HOME -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME \
        HOME="$HOME_G" PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_LIBRARY_PATH="$LIBS" \
        DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT_G" \
        LANG="$G_LANG" LANGUAGE="$G_LANGUAGE" \
        GTK_IM_MODULE=fcitx QT_IM_MODULE=fcitx XMODIFIERS=@im=fcitx \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS_G" \
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SYSBUS_G" \
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
    exec 0</dev/null 1>/dev/null 2>/dev/null
    nohup env -u XDG_CACHE_HOME -u XDG_CONFIG_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME \
        HOME="$HOME_G" PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" NR_LIBRARY_PATH="$LIBS" \
        DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT_G" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS_G" \
        DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SYSBUS_G" \
        LD_LIBRARY_PATH=/usr/lib/aarch64-linux-gnu:/usr/lib \
        G_RESOURCE_OVERLAYS=/org/gnome/shell=/usr/local/share/linuxhub/gnome-shell-overlay/org/gnome/shell \
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
i=0
while [ $i -lt 15 ]; do
    if g usr/bin/xwininfo -root -children 2>/dev/null | grep -q 'Desktop Icons'; then ding=yes; break; fi
    sleep 1; i=$((i+1))
done
# GPU 这一项只看证据，不猜：llvmpipe 会话的日志里会有 llvmpipe/swrast，zink+PanVK 的
# 会话里会出现 PanVK/FristOneRR 的告警。rootfs 里没有 gl-probe（那是开发机上的探针），
# 所以这里读日志而不是跑探针。
gpu=unknown
if [ -f "$ROOTFS/tmp/husk-shell.log" ]; then
    if grep -qiE 'llvmpipe|swrast' "$ROOTFS/tmp/husk-shell.log"; then gpu=llvmpipe
    elif grep -qiE 'panvk|FristOneRR|zink' "$ROOTFS/tmp/husk-shell.log"; then gpu=zink
    fi
fi

echo "CHECK wm=$wm"
echo "CHECK ding=$ding"
echo "CHECK gpu=$gpu"

# 5b. 输入预热（evidence 206）。会话起来之后的**第一次**指针 grab op 不会搬窗：
#     mutter 日志里 "grab op 1281 ... successful" → "Computing edges to resist-movement
#     or snap-to" → "Ending grab op"，位置一格没动；紧接着的第二次拖动就正常了。
#     用一次空点击 + 一次 8 px 的空拖动把这一次消耗掉，用户第一次拖窗口就是好的。
#     选点在屏幕中下部（远离左侧 dock 和 DING 的图标列），失败不影响会话。
input=no
dims=$(g usr/bin/xdpyinfo 2>/dev/null | awk '/dimensions:/ {print $2; exit}')
W=${dims%x*}; H=${dims#*x}
case "$W$H" in ''|*[!0-9]*) W=800; H=600 ;; esac
PX=$(( W / 2 )); PY=$(( H / 4 * 3 ))
if g usr/bin/xdotool mousemove "$PX" "$PY" >/dev/null 2>&1; then
    g usr/bin/xdotool click 1 >/dev/null 2>&1 || true
    g usr/bin/xdotool mousedown 1 >/dev/null 2>&1 || true
    g usr/bin/xdotool mousemove "$((PX + 8))" "$((PY + 8))" >/dev/null 2>&1 || true
    g usr/bin/xdotool mouseup 1 >/dev/null 2>&1 || true
    input=primed
fi
echo "CHECK input=$input"

# 5c. 中文输入法（evidence 210）。树里装了 fcitx5 和它的 gtk3/gtk4 immodule，但没有任何东西
#     去起它：结果就是中文能显示、不能输入。这里起在会话总线里，起不来不影响会话，
#     自检里如实写 im=none。
im=none
if [ -x "$ROOTFS/usr/bin/fcitx5" ]; then
    g_bg "$ROOTFS/tmp/fcitx5.log" usr/bin/fcitx5 -d --replace >/dev/null 2>&1 || true
    i=0
    while [ $i -lt 10 ]; do
        if g usr/bin/fcitx5-remote -o >/dev/null 2>&1; then im=fcitx5; break; fi
        sleep 1; i=$((i+1))
    done
fi
echo "CHECK im=$im"
echo "CHECK shell_log=$SHELL_LOG"

# 壁纸：装好的看护脚本会把当前壁纸缩到屏幕尺寸（并修复空/失效的 picture-uri）。
# 它可能和本脚本同级（开发机），也可能在 rootfs 里（安装后的位置），两个都找。
FIT=
for cand in "$BASE/husk-wallpaper-fit.sh" "$ROOTFS/usr/local/share/linuxhub/husk-wallpaper-fit.sh"; do
    [ -x "$cand" ] && { FIT=$cand; break; }
done
FIT=${FIT:-}
if [ -n "$FIT" ]; then
    DISPLAY="$DISPLAY" XDG_RUNTIME_DIR="$RT_G" DBUS_SESSION_BUS_ADDRESS="unix:path=$SESSBUS_G" \
        "$FIT" "$ROOTFS" "$NR_LD" "$HOME_G" >/dev/null 2>&1 &
fi

[ "$wm" = yes ] || { say "会话没起来：$SHELL_LOG"; exit 71; }
say "会话已起（wm=$wm ding=$ding gpu=$gpu）"
