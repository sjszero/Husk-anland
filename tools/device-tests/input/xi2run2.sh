#!/bin/sh
F=/data/user/0/com.linuxhub.myth.husk/files/linux
R=$F/distributions/ubuntu-24.04/rootfs
x() { DISPLAY=:0 sh $F/xe-husk.sh "$@" 2>/dev/null; }
run() {
  mode=$1
  rm -f $R/tmp/xi2-$mode.log
  nohup sh -c "DISPLAY=:0 sh $F/xe-husk.sh $R/tmp/xi2probe $mode 7" > $R/tmp/xi2-$mode.log 2>&1 &
  sleep 2
  W=$(x usr/bin/xdotool search --name xi2probe | head -1)
  eval "$(x usr/bin/xwininfo -id $W | sed -n 's/^  Absolute upper-left X: */X=/p;s/^  Absolute upper-left Y: */Y=/p;s/^  Width: */W=/p;s/^  Height: */H=/p')"
  echo "  win=$W at $X,$Y ${W}x${H}"
  CX=$((X + W/2)); CY=$((Y + H/2))
  x usr/bin/xdotool mousemove --sync $CX $CY
  sleep 0.3
  i=0; while [ $i -lt 6 ]; do
    x usr/bin/xdotool mousemove --sync $((CX + i*20)) $((CY + i*12))
    sleep 0.15; i=$((i+1))
  done
  x usr/bin/xdotool click 1
  sleep 0.3
  x usr/bin/xdotool mousemove --sync $((CX+60)) $((CY+40))
  sleep 5
  echo "== mode=$mode"
  grep -a "RESULT\|XIGrab\|XISelect\|win=" $R/tmp/xi2-$mode.log
  grep -a "segv" $R/tmp/xi2-$mode.log | head -2
}
run nograb
run grab
