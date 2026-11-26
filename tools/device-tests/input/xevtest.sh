#!/bin/sh
F=/data/user/0/com.linuxhub.husk/files/linux
R=$F/distributions/ubuntu-24.04/rootfs
x() { DISPLAY=:0 sh $F/xe-husk.sh "$@" 2>/dev/null; }
rm -f $R/tmp/xev.log
nohup sh -c "DISPLAY=:0 sh $F/xe-husk.sh usr/bin/xev -geometry 300x200+1500+300" > $R/tmp/xev.log 2>&1 &
sleep 6
W=$(x usr/bin/xdotool search --name "xev" | head -1)
echo "xev win=$W"
[ -n "$W" ] && x usr/bin/xdotool windowmove $W 1500 300
sleep 1
x usr/bin/xdotool mousemove --sync 1650 400
sleep 0.3
x usr/bin/xdotool click 1
sleep 0.3
x usr/bin/xdotool mousemove --sync 1700 430
sleep 0.5
echo "== xev log (事件摘要)"
grep -aoE "^(ButtonPress|ButtonRelease|MotionNotify|EnterNotify|LeaveNotify|KeyPress|MapNotify)" $R/tmp/xev.log | sort | uniq -c
echo "== 原始前 30 行"
head -30 $R/tmp/xev.log
