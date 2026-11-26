#!/bin/sh
F=/data/user/0/com.linuxhub.husk/files/linux
R=$F/distributions/ubuntu-24.04/rootfs
x() { DISPLAY=:0 sh $F/xe-husk.sh "$@" 2>/dev/null; }
cg() { x usr/bin/xwininfo -id "$1" 2>/dev/null | sed -n 's/^  Absolute upper-left X: *\([-0-9]*\).*/X=\1/p;s/^  Absolute upper-left Y: *\([-0-9]*\).*/Y=\1/p;s/^  Width: *\([0-9]*\).*/W=\1/p;s/^  Height: *\([0-9]*\).*/H=\1/p' | tr '\n' ' '; }
W=$(x usr/bin/xdotool search --name mt3 2>/dev/null | head -1)
[ -n "$W" ] || { echo "no window"; exit 2; }
eval "$(cg $W)"; echo "client=$W at $X,$Y ${W}x${H}"
FE=$(x usr/bin/xprop -id $W _NET_FRAME_EXTENTS 2>/dev/null)
echo "extents: $FE"
TOP=$(echo "$FE" | sed -n 's/.*= *[0-9]*, *[0-9]*, *\([0-9]*\),.*/\1/p')
[ -n "$TOP" ] || TOP=49
TBX=$((X + W/2)); TBY=$((Y - TOP/2))
echo "== 按标题栏 ($TBX,$TBY)   [top extent=$TOP]"
x usr/bin/xdotool mousemove --sync $TBX $TBY
sleep 0.4
x usr/bin/xdotool getmouselocation
x usr/bin/xdotool mousedown 1
sleep 0.4
x usr/bin/xdotool mousemove --sync $((TBX+80)) $((TBY+60)); sleep 0.25
x usr/bin/xdotool mousemove --sync $((TBX+260)) $((TBY+180)); sleep 0.6
x usr/bin/xdotool mouseup 1
sleep 0.8
eval "$(cg $W)"; echo "after titlebar-drag: $X,$Y"
echo "== 再试客户端消息路径（在客户端区按下）"
CX=$((X + W/2)); CY=$((Y + H/2))
x usr/bin/xdotool mousemove --sync $CX $CY; sleep 0.3
x usr/bin/xdotool mousedown 1; sleep 0.3
x usr/bin/xdotool mousemove --sync $((CX+60)) $((CY+30)); sleep 0.2
x usr/bin/xdotool mousemove --sync $((CX+200)) $((CY+120)); sleep 0.5
x usr/bin/xdotool mouseup 1; sleep 0.6
eval "$(cg $W)"; echo "after client-msg drag: $X,$Y"
echo "== 探针：抓取与 X 错误（最近 20 条）"
grep -aE "XIGrabDevice|XERROR|XGrabPointer|XGrabKeyboard" $R/tmp/xigrab.log | tail -20
