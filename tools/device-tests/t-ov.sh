#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
export DISPLAY=:0
xdotool mousemove 24 1039
sleep 0.3
xdotool click 1
sleep 2
echo clicked
