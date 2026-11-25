#!/bin/sh
F=/data/user/0/com.linuxhub.myth.husk/files/linux
R=$F/distributions/ubuntu-24.04/rootfs
x() { DISPLAY=:0 sh $F/xe-husk.sh "$@" 2>/dev/null; }
W=0x2000001
pos() { x usr/bin/xwininfo -id $W | sed -n 's/^  Absolute upper-left X: *//p;s/^  Absolute upper-left Y: *//p' | tr '\n' ' '; echo; }
echo "before: $(pos)"
x usr/bin/xdotool windowactivate $W; sleep 0.5
x usr/bin/xdotool key --clearmodifiers alt+F7; sleep 0.6
echo "-- 连按 20 次 Right"
i=0; while [ $i -lt 20 ]; do x usr/bin/xdotool key --clearmodifiers Right; i=$((i+1)); done
sleep 0.4
echo "-- 连按 10 次 Down"
i=0; while [ $i -lt 10 ]; do x usr/bin/xdotool key --clearmodifiers Down; i=$((i+1)); done
sleep 0.4
x usr/bin/xdotool key --clearmodifiers Return
sleep 0.8
echo "after : $(pos)"
