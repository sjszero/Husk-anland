#!/bin/bash
export PATH=/usr/bin:/bin:$PATH
echo "DISPLAY=${DISPLAY:-unset}  XDG_SESSION_TYPE=${XDG_SESSION_TYPE:-unset}"
ls -l /tmp/.X11-unix/ 2>&1 | head -3
for i in 1 2 3; do
  s=$(date +%s%N); DISPLAY=:0 /usr/bin/glxinfo -B >/tmp/t357-glx.txt 2>&1; e=$(date +%s%N)
  echo "glxinfo-B(DISPLAY=:0) run$i: $(( (e-s)/1000000 )) ms"
done
grep -E "OpenGL renderer|OpenGL version|direct rendering" /tmp/t357-glx.txt
echo "---- raw vulkan probe (no X) ----"
for i in 1 2; do
  s=$(date +%s%N); env -u DISPLAY /usr/bin/vulkan-probe >/dev/null 2>&1; e=$(date +%s%N)
  echo "vulkan-probe run$i: $(( (e-s)/1000000 )) ms"
done
echo "---- gl_probe readback 16x16 ----"
s=$(date +%s%N); /usr/bin/gl_probe readback 16x16 2>&1 | tail -3; e=$(date +%s%N)
echo "gl_probe total: $(( (e-s)/1000000 )) ms"
