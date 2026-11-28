#!/bin/sh
# 19 · Run the complete 19-case Rust readback suite through the real Husk loader.
#
# Why this is separate from 18-gpu-loader-defaults.sh: 18 proves which renderer a
# non-session process inherits, but it only calls `gl-probe readback 16x16`. The
# round-5 contract also covers row-width boundaries, canary tails, whole-block
# reads and the row-by-row shape used by screenshots. This script makes that
# complete suite an actual device gate instead of a one-off command in evidence.
#
#   DEV=UKHMMB8PYTGECMNV make readback-audit
#
# Exit 0 only when the probe exits 0, reports PASS=19 FAIL=0, and the renderer
# matches EXPECT_RENDERER (default: zink). Set EXPECT_RENDERER= for a software-only
# diagnostic run; that run does not satisfy the hardware presentation gate.
set -u
DEV=${DEV:?usage: DEV=<adb serial> [ROOTFS=...] [LD=...] $0}
PKG=${PKG:-com.linuxhub.myth.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
HERE=$(cd "$(dirname "$0")" && pwd)
GL_PROBE=${GL_PROBE:-$HERE/../../runtime/target/aarch64-unknown-linux-gnu/release/gl-probe}
EXPECT_PASS=${EXPECT_PASS:-19}
EXPECT_RENDERER=${EXPECT_RENDERER-zink}
LOG=${LOG:-}
PASS=0
FAIL=0

ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

push() { # push <local> <guest-relative-in-tree>
    adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $ROOTFS/$2'" < "$1"
}

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

if [ "$FAIL" -ne 0 ]; then
    echo "READBACK_AUDIT PASS=$PASS FAIL=$FAIL"
    exit 1
fi

echo "== push Rust probe =="
if push "$HERE/husk-exec.sh" tmp/husk-exec.sh &&
   push "$GL_PROBE" root/gl-probe &&
   adb_ "run-as $PKG sh -c 'chmod 755 $ROOTFS/root/gl-probe $ROOTFS/tmp/husk-exec.sh'" >/dev/null; then
    ok "probe and launcher pushed"
else
    bad "could not push probe or launcher"
    echo "READBACK_AUDIT PASS=$PASS FAIL=$FAIL"
    exit 1
fi

echo "== 19-case readback suite through the loader =="
out=$(adb_ "run-as $PKG sh $ROOTFS/tmp/husk-exec.sh $ROOTFS $LD root/gl-probe readback-suite" 2>&1)
rc=$?
printf '%s\n' "$out"
if [ -n "$LOG" ]; then
    printf '%s\n' "$out" > "$LOG"
fi

if [ "$rc" -eq 0 ]; then
    ok "probe exited 0"
else
    bad "probe exited $rc"
fi

renderer=$(printf '%s\n' "$out" | sed -n 's/^TAG renderer=//p' | tail -1)
echo "  STATE renderer: ${renderer:-<empty>}"
if [ -z "$EXPECT_RENDERER" ]; then
    ok "renderer check skipped explicitly"
else
    case "$renderer" in
        *"$EXPECT_RENDERER"*) ok "renderer contains $EXPECT_RENDERER" ;;
        *) bad "renderer does not contain $EXPECT_RENDERER" ;;
    esac
fi

counts=$(printf '%s\n' "$out" | sed -n 's/^PASS=\([0-9][0-9]*\) FAIL=\([0-9][0-9]*\)$/\1 \2/p' | tail -1)
if [ -z "$counts" ]; then
    bad "probe printed no final PASS/FAIL summary"
else
    set -- $counts
    got_pass=$1
    got_fail=$2
    echo "  STATE summary: PASS=$got_pass FAIL=$got_fail expected PASS=$EXPECT_PASS FAIL=0"
    if [ "$got_pass" -eq "$EXPECT_PASS" ] && [ "$got_fail" -eq 0 ]; then
        ok "complete readback suite passed"
    else
        bad "readback suite counts do not match the release contract"
    fi
fi

echo "READBACK_AUDIT PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
