#!/bin/sh
# 21 · Multi-process GPU soak through the real guest loader.
#
# This is the process-level long-run gate that complements 19-readback-suite.sh:
# each job creates its own zink/PanVK context and repeatedly clears a 2400x1080
# pbuffer with glFinish. The script checks renderer, exit status, GL errors, fd
# counts and RSS bounds for every job. It does not claim a desktop result;
# desktop/X11 readiness is a separate host-integration check.
#
#   DEV=<serial> SOAK_SECONDS=300 JOBS=4 make gpu-soak
set -u
DEV=${DEV:?usage: DEV=<adb serial> [ROOTFS=...] [LD=...] $0}
PKG=${PKG:-com.linuxhub.myth.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
GL_PROBE=${GL_PROBE:-$HERE/../../runtime/target/aarch64-unknown-linux-gnu/release/gl-probe}
SOAK_SECONDS=${SOAK_SECONDS:-300}
JOBS=${JOBS:-4}
SIZE=${SIZE:-2400x1080}
RSS_LIMIT_KB=${RSS_LIMIT_KB:-65536}
PASS=0
FAIL=0
SKIP=0

ok()   { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
skip() { SKIP=$((SKIP+1)); echo "  SKIP  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

push() {
    adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $ROOTFS/$2'" < "$1"
}

case "$SOAK_SECONDS" in ''|*[!0-9]*) bad "SOAK_SECONDS must be a positive integer"; exit 1 ;; esac
case "$JOBS" in ''|*[!0-9]*) bad "JOBS must be a positive integer"; exit 1 ;; esac
[ "$SOAK_SECONDS" -ge 1 ] || { bad "SOAK_SECONDS must be >= 1"; exit 1; }
[ "$JOBS" -ge 1 ] || { bad "JOBS must be >= 1"; exit 1; }

echo "== prerequisites =="
if adb_ "run-as $PKG sh -c 'test -x $ROOTFS/usr/bin/bash && test -r $LD && echo yes'" 2>/dev/null | grep -qx yes; then
    ok "guest bash and loader are readable"
else
    bad "missing guest bash or loader (ROOTFS=$ROOTFS LD=$LD)"
fi
if [ -x "$GL_PROBE" ]; then
    ok "host gl-probe exists"
else
    bad "missing $GL_PROBE (run tools/guest-config/build-guest-tools.sh)"
fi
[ "$FAIL" -eq 0 ] || { echo "GPU_SOAK_AUDIT PASS=$PASS FAIL=$FAIL SKIP=$SKIP"; exit 1; }

echo "== push Rust soak probe =="
if push "$HERE/husk-exec.sh" tmp/husk-exec.sh &&
   push "$GL_PROBE" root/gl-probe &&
   adb_ "run-as $PKG sh -c 'chmod 755 $ROOTFS/root/gl-probe $ROOTFS/tmp/husk-exec.sh'" >/dev/null; then
    ok "probe and launcher pushed"
else
    bad "could not push probe or launcher"
    echo "GPU_SOAK_AUDIT PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
    exit 1
fi

# Run four independent loader invocations concurrently. Each adb shell stays
# attached to its job, so a failed job cannot be mistaken for a completed one.
echo "== ${JOBS} jobs for ${SOAK_SECONDS}s at ${SIZE} =="
pids=""
for n in $(seq 1 "$JOBS"); do
    (adb_ "run-as $PKG sh $ROOTFS/tmp/husk-exec.sh $ROOTFS $LD root/gl-probe soak $SIZE $SOAK_SECONDS p$n" >"/tmp/husk-gpu-soak-p$n.log" 2>&1) &
    pids="$pids $!"
done
for p in $pids; do
    if wait "$p"; then :; else bad "job pid $p exited nonzero"; fi
done

for n in $(seq 1 "$JOBS"); do
    log=/tmp/husk-gpu-soak-p$n.log
    if [ ! -s "$log" ]; then
        bad "job $n produced no log"
        continue
    fi
    renderer=$(sed -n 's/^TAG p[0-9][0-9]* GL_RENDERER=//p' "$log" | head -1)
    echo "  STATE p$n renderer: ${renderer:-<empty>}"
    case "$renderer" in
        *zink*Mali-G77*) ok "job $n used zink/Mali" ;;
        *llvmpipe*) bad "job $n fell back to llvmpipe" ;;
        *) bad "job $n has unexpected renderer" ;;
    esac
    done_line=$(grep -E "^DONE p${n} " "$log" | tail -1)
    if [ -z "$done_line" ]; then
        bad "job $n did not print DONE"
    elif printf '%s\n' "$done_line" | grep -q 'errors=0'; then
        ok "job $n completed with errors=0"
    else
        bad "job $n reported GL errors"
    fi
    start_fd=$(sed -n 's/^START p[0-9][0-9]* .* fd=\([0-9][0-9]*\) mali_fd=.*/\1/p' "$log" | head -1)
    done_fd=$(printf '%s\n' "$done_line" | sed -n 's/.* fd=\([0-9][0-9]*\) (start [0-9][0-9]*).*/\1/p')
    if [ -n "$start_fd" ] && [ -n "$done_fd" ] && [ "$start_fd" -eq "$done_fd" ]; then
        ok "job $n fd count stable ($start_fd)"
    else
        bad "job $n fd count changed (start=${start_fd:-?} end=${done_fd:-?})"
    fi
    start_rss=$(sed -n 's/^START p[0-9][0-9]* .* rss=\([0-9][0-9]*\)kB.*/\1/p' "$log" | head -1)
    done_rss=$(printf '%s\n' "$done_line" | sed -n 's/.*rss=\([0-9][0-9]*\)kB (start [0-9][0-9]*kB).*/\1/p')
    if [ -n "$start_rss" ] && [ -n "$done_rss" ]; then
        delta=$((done_rss - start_rss))
        absolute_delta=$delta
        if [ "$absolute_delta" -lt 0 ]; then absolute_delta=$((-absolute_delta)); fi
        if [ "$absolute_delta" -le "$RSS_LIMIT_KB" ]; then
            ok "job $n RSS bounded (delta=${delta}kB)"
        else
            bad "job $n RSS changed by ${delta}kB (absolute limit=${RSS_LIMIT_KB}kB)"
        fi
    else
        bad "job $n RSS start/end missing"
    fi
done

echo "GPU_SOAK_AUDIT PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
[ "$FAIL" -eq 0 ]
