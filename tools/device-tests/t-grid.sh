#!/bin/bash
# open the app grid (bottom-left Ubuntu/apps button) and leave it open
export PATH=/usr/bin:/bin:$PATH
export DISPLAY=:0
xdotool key Escape 2>/dev/null
sleep 1
xdotool mousemove 24 1039
sleep 0.3
xdotool click 1
sleep 3
echo opened
