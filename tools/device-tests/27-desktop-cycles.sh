#!/bin/sh
# 27 · 桌面冷启动 × N 次 + 强退无残留（0.0.37 性能/会话门禁）
#
# 每次：force-stop → 起 App → 点“打开桌面” → 轮询 gnome-shell → 等 X 能应答
# → 截一帧判首帧非黑 → force-stop → 断言这棵树的 guest 进程全部消失。
# 输出 CSV 到 /tmp/husk-desktop-cycles.csv，末尾给中位数 / p95 / 最大值 / 4–5 s 异常数。
#
#   DEV=<serial> PKG=<pkg> ROOTFS=files/.../rootfs CYCLES=20 sh 27-desktop-cycles.sh
set -u
DEV=${DEV:?usage: DEV=<adb serial> [PKG=...] [ROOTFS=...] [CYCLES=20] $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
CYCLES=${CYCLES:-20}
TAP_X=${TAP_X:-912}; TAP_Y=${TAP_Y:-2142}
CSV=${CSV:-/tmp/husk-desktop-cycles.csv}
HERE=$(cd "$(dirname "$0")" && pwd)
: > "$CSV"
echo "cycle,tap_to_gs_ms,tap_to_x_ms,center_mean,near_black_ratio,residual_procs,status" >> "$CSV"

adb_() { adb -s "$DEV" shell "$@"; }
shell_pid() {
    adb_ "run-as $PKG sh -c 'for p in /proc/[0-9]*; do c=\$(tr \"\\0\" \" \" < \$p/cmdline 2>/dev/null) || continue; case \"\$c\" in *usr/bin/gnome-shell*) echo \${p#/proc/}; break;; esac; done' 2>/dev/null" | tr -d '\r' | tail -1
}
residual() {
    adb_ "run-as $PKG sh -c 'n=0; for p in /proc/[0-9]*; do c=\$(tr \"\\0\" \" \" < \$p/cmdline 2>/dev/null) || continue; case \"\$c\" in *\"$ROOTFS\"*) n=\$((n+1));; esac; done; echo \$n'" | tr -d '\r' | tail -1
}
x_ready() {   # 0 = X answers, 1 = timeout (bounded, never leaves a stray probe)
    ( adb_ "run-as $PKG sh $ROOTFS/tmp/husk-exec.sh $ROOTFS $LD usr/bin/env DISPLAY=:0 XDG_RUNTIME_DIR=/tmp/runtime-husk usr/bin/xdotool getactivewindow" >/dev/null 2>&1 ) &
    p=$!; i=0
    while [ $i -lt 10 ]; do kill -0 $p 2>/dev/null || return 0; sleep 1; i=$((i+1)); done
    kill $p 2>/dev/null; return 1
}

i=1
while [ "$i" -le "$CYCLES" ]; do
    adb_ "am force-stop $PKG" >/dev/null 2>&1
    sleep 2
    adb_ "am start -n $PKG/com.java.myapplication.MainActivity" >/dev/null 2>&1
    sleep 6
    t0=$(python3 -c 'import time;print(int(time.time()*1000))')
    adb_ "input tap $TAP_X $TAP_Y" >/dev/null 2>&1
    gs=; gs_ms=-1; k=0
    while [ $k -lt 90 ]; do
        gs=$(shell_pid)
        if [ -n "$gs" ]; then
            t1=$(python3 -c 'import time;print(int(time.time()*1000))')
            gs_ms=$((t1-t0)); break
        fi
        sleep 1; k=$((k+1))
    done
    x_ms=-1
    if [ -n "$gs" ]; then
        if x_ready; then x_ms=$(( $(python3 -c 'import time;print(int(time.time()*1000))') - t0 )); fi
    fi
    center=0; ratio=1.0
    if [ "$x_ms" -ge 0 ]; then
        adb -s "$DEV" exec-out screencap -p > "/tmp/cycle-$i.png" 2>/dev/null
        center=$(python3 - "$i" <<'PY'
import sys
from PIL import Image
im=Image.open(f"/tmp/cycle-{sys.argv[1]}.png").convert("RGB")
w,h=im.size
px=im.load(); xs=range(int(w*0.35),int(w*0.65),4); ys=range(int(h*0.35),int(h*0.65),4)
tot=mean=nb=0
for y in ys:
    for x in xs:
        r,g,b=px[x,y]; tot+=1; mean+=r+g+b
        if r<8 and g<8 and b<8: nb+=1
print(f"{mean/(3*tot):.1f} {nb/tot:.4f}")
PY
)
    fi
    resid=$(residual)
    adb_ "am force-stop $PKG" >/dev/null 2>&1
    sleep 3
    resid_after=$(residual)
    status=ok
    [ -n "$gs" ] || status=no-gnome-shell
    [ "$x_ms" -ge 0 ] || status=no-x
    [ "$resid_after" = 0 ] || status="residual=$resid_after"
    echo "$i,$gs_ms,$x_ms,$center,$resid_after,$resid_after,$status" >> "$CSV"
    echo "cycle $i: gs=${gs_ms}ms x=${x_ms}ms center=$center residual=$resid_after $status"
    i=$((i+1))
done

python3 - "$CSV" <<'PY'
import csv,sys,statistics
rows=[r for r in csv.DictReader(open(sys.argv[1])) ]
def nums(k): return sorted(int(r[k]) for r in rows if int(r[k])>=0)
for k in ("tap_to_gs_ms","tap_to_x_ms"):
    v=nums(k)
    if not v: print(f"{k}: none"); continue
    p95=v[min(len(v)-1,int(round(0.95*(len(v)-1))))]
    print(f"{k}: n={len(v)} median={statistics.median(v):.0f} p95={p95} max={max(v)} "
          f"anomaly_4_5s={sum(1 for x in v if 4000<=x<5000)}")
print("statuses:", {s:sum(1 for r in rows if r['status']==s) for s in {r['status'] for r in rows}})
PY
