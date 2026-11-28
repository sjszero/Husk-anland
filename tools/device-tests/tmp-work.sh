#!/bin/sh
# 同一套日常负载，自包含；输出一行一个指标（毫秒）
cd /root || exit 1
python3 - <<'PY'
import os, time, subprocess
def t(fn, n=1):
    best=[]
    for _ in range(n):
        t0=time.time(); fn(); best.append(time.time()-t0)
    return min(best)*1000
# stat 20000
def st():
    for i in range(20000):
        try: os.stat('/usr/bin/bash')
        except OSError: pass
print("stat_20000_ms=%.1f" % t(st))
def oc():
    for i in range(20000):
        fd=os.open('/usr/bin/bash', os.O_RDONLY); os.close(fd)
print("open_close_20000_ms=%.1f" % t(oc))
def loop():
    for i in range(1000):
        pass
print("loop_1000_ms=%.1f" % t(loop))
PY
# tar: 解 200 个源文件
rm -rf /tmp/bx && mkdir -p /tmp/bx && cp -r /root/bench-make /tmp/bx/src 2>/dev/null
if [ -d /tmp/bx/src ]; then
  tar cf /tmp/bx/a.tar -C /tmp/bx/src . 2>/dev/null
  s=$(date +%s%3N); tar xf /tmp/bx/a.tar -C /tmp/bx/out 2>/dev/null || { mkdir -p /tmp/bx/out; tar xf /tmp/bx/a.tar -C /tmp/bx/out; }
  e=$(date +%s%3N); echo "tar_extract_ms=$((e-s))"
else
  echo "tar_extract_ms=NA(no fixture)"
fi
# /bin/true ×50
s=$(date +%s%3N); i=0; while [ $i -lt 50 ]; do /bin/true; i=$((i+1)); done; e=$(date +%s%3N)
echo "true_50_ms=$((e-s))"
# make -j4
if [ -d /root/bench-make ]; then
  cd /root/bench-make && rm -f *.o app 2>/dev/null
  s=$(date +%s%3N); make -j4 app >/dev/null 2>&1; e=$(date +%s%3N); echo "make_j4_ms=$((e-s))"
else
  echo "make_j4_ms=NA(no fixture)"
fi
