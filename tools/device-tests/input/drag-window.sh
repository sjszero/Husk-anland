#!/bin/sh
# drag-window.sh - 拖窗对照：mutter 的指针搬窗 vs alt+F7（evidence 197/198/203/205）
#
# usage: drag-window.sh <rootfs-abs> [loader-abs] [extra-libpath]
#   例（本机开发布局）:
#     sh drag-window.sh /data/user/0/com.linuxhub.myth.husk/files/linux/distributions/ubuntu-24.04/rootfs \
#        /data/user/0/com.linuxhub.myth.husk/files/linux/husk/libld.so
#
# 期望输出（0.0.6 实测）：Super+拖前后坐标一字不差，alt+F7 之后坐标变化。
# 需要：X server 在 :0 上活着、guest 里有 xev/xdotool/xwininfo。
# 注意：屏幕一灭，应用会被 Android 冻结，本脚本会全卡住（evidence 203 §3），
#       跑之前先唤醒，跑的期间每 15 秒 `input keyevent KEYCODE_WAKEUP` 保活。
set -eu
R=${1:?usage: drag-window.sh <rootfs-abs> [loader-abs] [extra-libpath]}
LD=${2:-$(dirname "$R")/libld.so}
EXTRA=${3:-}
LIBS="$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu"
for d in mutter-14 gnome-shell; do
    [ -d "$R/usr/lib/aarch64-linux-gnu/$d" ] && LIBS="$LIBS:$R/usr/lib/aarch64-linux-gnu/$d"
done
[ -d "$R/usr/lib/gnome-shell" ] && LIBS="$LIBS:$R/usr/lib/gnome-shell"
[ -n "$EXTRA" ] && LIBS="$LIBS:$EXTRA"

X() {
    ( cd "$R" && env -i HOME=/root PATH=/usr/bin:/bin \
        NR_ROOTFS="$R" NR_LD="$LD" NR_LIBRARY_PATH="$LIBS" DISPLAY=:0 \
        "$LD" --library-path "$LIBS" "$@" ) </dev/null
}

X usr/bin/xev -geometry 600x500+250+200 >/dev/null 2>&1 &
sleep 8
W=$(X usr/bin/xwininfo -root -children 2>/dev/null | awk '/"Event Tester"/ {print $1; exit}')
echo "win=$W"
[ -n "$W" ] || { echo "没有找到 xev 窗口"; exit 1; }

geo() { X usr/bin/xwininfo -id "$W" | awk '/Absolute upper-left X|Absolute upper-left Y|Width:|Height:/ {printf "%s ", $NF}'; echo; }

X usr/bin/xdotool windowactivate "$W" >/dev/null 2>&1 || true
sleep 1
echo "geo_before: $(geo)"

eval "$(X usr/bin/xdotool getwindowgeometry --shell "$W")"
CX=$((X + WIDTH/2)); CY=$((Y + HEIGHT/2))
X usr/bin/xdotool mousemove $CX $CY; sleep 1
X usr/bin/xdotool keydown super; sleep 0.3
X usr/bin/xdotool mousedown 1; sleep 0.5
X usr/bin/xdotool mousemove $((CX+120)) $((CY+90)); sleep 0.5
X usr/bin/xdotool mousemove $((CX+240)) $((CY+160)); sleep 0.5
X usr/bin/xdotool mouseup 1; sleep 0.3
X usr/bin/xdotool keyup super; sleep 1.5
echo "geo_after_super_drag: $(geo)"

X usr/bin/xdotool key alt+F7; sleep 1.2
X usr/bin/xdotool mousemove 500 400; sleep 1
X usr/bin/xdotool click 1; sleep 1.5
echo "geo_after_alt_f7: $(geo)"
