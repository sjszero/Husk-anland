#!/bin/sh
# 22 · Embedded X11 desktop acceptance.
#
# This gate starts only com.linuxhub.husk (never another LinuxHub package), waits
# for the guest launcher's readiness contract, rejects a black/large-missing
# frame, verifies the capability matrix says x11=ready, and proves that a real
# Android tap/key reached Xlorie. A device-health preflight refuses to start a
# 3-process-per-thread runaway phone just to produce a misleading result.
#
#   DEV=<serial> ./22-desktop-acceptance.sh
#
# Exit: 0 PASS, 1 test failure, 2 prerequisites/device health blocked.
set -u
DEV=${DEV:?usage: DEV=<adb serial> $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
HUSK_TOOLS=${HUSK_TOOLS:-$REPO_ROOT/runtime/target/debug/husk-tools}
MAX_LOAD=${MAX_LOAD:-12}
MAX_THREADS=${MAX_THREADS:-12000}
MIN_MEM_KB=${MIN_MEM_KB:-2097152}
WAIT_SECS=${WAIT_SECS:-90}
PASS=0; FAIL=0; SKIP=0
TMP=${TMPDIR:-/tmp}/husk-desktop-acceptance-$$
mkdir -p "$TMP" || exit 2
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
ok()   { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
skip() { SKIP=$((SKIP+1)); echo "  SKIP  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

echo "== prerequisites =="
state=$(adb -s "$DEV" get-state 2>/dev/null || true)
if [ "$state" = device ]; then ok "adb device is online"; else
    bad "adb device is not online (state=${state:-missing})"
    echo "DESKTOP_ACCEPTANCE PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
    exit 2
fi
[ -x "$HUSK_TOOLS" ] || { bad "missing $HUSK_TOOLS (run make tools)"; exit 2; }
if adb_ "pm path $PKG" 2>/dev/null | grep -q '^package:'; then
    ok "$PKG is installed"
else
    bad "$PKG is not installed"
    exit 2
fi

load1=$(adb_ 'cat /proc/loadavg' 2>/dev/null | awk '{print $1}')
case "$load1" in ''|*[!0-9.]*) load1=999 ;; esac
threads=$(adb_ 'ps -e -T 2>/dev/null | wc -l' 2>/dev/null | tr -d ' \r')
mem_kb=$(adb_ 'grep ^MemAvailable: /proc/meminfo' 2>/dev/null | awk '{print $2}')
case "$threads" in ''|*[!0-9]*) threads=999999 ;; esac
case "$mem_kb" in ''|*[!0-9]*) mem_kb=0 ;; esac
echo "  STATE load1=$load1 threads=$threads MemAvailable=${mem_kb}kB"
if awk -v n="$load1" -v max="$MAX_LOAD" 'BEGIN { exit !(n <= max) }'; then
    ok "load1 is below the safety ceiling ($MAX_LOAD)"
else
    bad "load1=$load1 exceeds safety ceiling $MAX_LOAD; refusing to start"
fi
if [ "$threads" -le "$MAX_THREADS" ]; then
    ok "process/thread count is below the safety ceiling ($MAX_THREADS)"
else
    bad "process/thread count=$threads exceeds safety ceiling $MAX_THREADS; refusing to start"
fi
if [ "$mem_kb" -ge "$MIN_MEM_KB" ]; then
    ok "MemAvailable is at least $MIN_MEM_KB kB"
else
    bad "MemAvailable=${mem_kb}kB is below $MIN_MEM_KB kB; refusing to start"
fi
if [ "$FAIL" -ne 0 ]; then
    echo "DESKTOP_ACCEPTANCE PASS=$PASS FAIL=$FAIL SKIP=$SKIP (BLOCKED: device health)"
    exit 2
fi

echo "== restart only $PKG =="
adb -s "$DEV" shell am force-stop "$PKG" >/dev/null 2>&1 || true
adb_ "run-as $PKG rm -f $ROOTFS/tmp/husk-desktop.status" >/dev/null 2>&1 || true
adb -s "$DEV" shell logcat -b main -c >/dev/null 2>&1 || true
if adb_ "am start -W -n $PKG/android.app.NativeActivity" >/dev/null 2>&1; then
    ok "NativeActivity launch accepted"
else
    bad "NativeActivity launch failed"
fi

echo "== readiness contract =="
status=
i=0
while [ "$i" -lt "$WAIT_SECS" ]; do
    status=$(adb_ "run-as $PKG cat $ROOTFS/tmp/husk-desktop.status" 2>/dev/null || true)
    first=$(printf '%s\n' "$status" | sed -n '1p')
    [ "$first" = ready ] && break
    [ "$first" = failed ] && break
    sleep 1
    i=$((i+1))
done
printf '%s\n' "$status" | sed 's/^/  STATE status: /'
first=$(printf '%s\n' "$status" | sed -n '1p')
getfield() { printf '%s\n' "$status" | sed -n "s/^$1=//p" | head -1; }
wm=$(getfield wm); ding=$(getfield ding); gpu=$(getfield gpu); input=$(getfield input); im=$(getfield im)
[ "$first" = ready ] && ok "launcher status is ready" || bad "launcher did not become ready (first=${first:-<timeout>})"
[ "$wm" = yes ] && ok "window manager is ready" || bad "window manager is not ready (wm=${wm:-?})"
[ "$ding" = yes ] && ok "desktop icons are ready" || bad "desktop icons are missing (ding=${ding:-?})"
[ "$gpu" = zink ] && ok "GNOME shell reports zink hardware GPU" || bad "GNOME shell is not fully on zink (gpu=${gpu:-?})"
[ "$input" = primed ] && ok "input grab path was primed" || bad "input path was not primed (input=${input:-?})"
[ "$im" = fcitx5 ] && ok "fcitx5 input method is running" || skip "fcitx5 is not running (im=${im:-?})"
[ "$first" = ready ] || { echo "DESKTOP_ACCEPTANCE PASS=$PASS FAIL=$FAIL SKIP=$SKIP"; exit 1; }

echo "== capability matrix =="
caps=$(adb_ "run-as $PKG cat files/husk-capabilities.txt" 2>/dev/null || true)
display_line=$(printf '%s\n' "$caps" | grep '^caps|display|' | head -1)
printf '%s\n' "$display_line" | grep -q 'x11=ready' \
    && ok "capability matrix says x11=ready" \
    || bad "capability matrix does not say x11=ready (${display_line:-<missing>})"

echo "== log readiness / MIT-SHM =="
log=$(adb -s "$DEV" logcat -d -s Husk:I '*:S' 2>/dev/null || true)
printf '%s\n' "$log" | grep -q 'desktop: ready' \
    && ok "logcat records desktop: ready" \
    || bad "logcat has no desktop: ready"
printf '%s\n' "$log" | grep -q 'mit_shm=disabled' \
    && ok "logcat records MIT-SHM disabled" \
    || bad "logcat does not record mit_shm=disabled"
printf '%s\n' "$log" | grep -q 'Failed to attach to x11 shm' \
    && bad "Xlorie logged Failed to attach to x11 shm" \
    || ok "no x11 shm attachment failure"

echo "== foreground and frame =="
focus=$(adb_ 'dumpsys activity activities' 2>/dev/null | grep -m1 -E 'topResumedActivity|mResumedActivity' || true)
case "$focus" in
    *"$PKG"*) ok "Husk is the resumed activity" ;;
    *) bad "Husk is not the resumed activity (${focus:-<no focus line>})" ;;
esac
adb_ 'input keyevent KEYCODE_WAKEUP' >/dev/null 2>&1 || true
sleep 1
if adb -s "$DEV" exec-out screencap > "$TMP/desktop.raw" 2>"$TMP/screencap.err"; then
    ok "raw screencap captured ($(wc -c < "$TMP/desktop.raw" | tr -d ' ') bytes)"
    if "$HUSK_TOOLS" screenshot-audit --strict "$TMP/desktop.raw"; then
        ok "frame is not all-black and has no large missing region"
    else
        bad "frame looks black or has a large missing region"
    fi
else
    bad "raw screencap failed: $(tr '\n' ' ' < "$TMP/screencap.err" | head -c 240)"
fi

echo "== Android input reaches Xlorie =="
adb -s "$DEV" shell logcat -b main -c >/dev/null 2>&1 || true
size=$(adb_ 'wm size' 2>/dev/null | sed -n 's/.*: \([0-9]*\)x\([0-9]*\).*/\1 \2/p' | tail -1)
sw=$(printf '%s\n' "$size" | awk '{print $1}')
sh=$(printf '%s\n' "$size" | awk '{print $2}')
case "${sw:-}:${sh:-}" in
    [0-9]*:[0-9]*) ;;
    *) sw=540; sh=1200 ;;
esac
[ -n "${sw:-}" ] && [ "$sw" -gt 0 ] || sw=540
[ -n "${sh:-}" ] && [ "$sh" -gt 0 ] || sh=1200
tx=$((sw / 2)); ty=$((sh / 2))
adb_ "input tap $tx $ty" >/dev/null 2>&1 || true
adb_ 'input keyevent 30' >/dev/null 2>&1 || true
sleep 2
ilog=$(adb -s "$DEV" logcat -d -s Husk:I '*:S' 2>/dev/null || true)
printf '%s\n' "$ilog" | grep -q 'desktop|input|touch' \
    && ok "tap reached the Xlorie touch path at ${tx}x${ty}" \
    || bad "no desktop|input|touch event after tap"
printf '%s\n' "$ilog" | grep -q 'desktop|input|key' \
    && ok "key event reached the Xlorie key path" \
    || bad "no desktop|input|key event after keyevent"

echo "DESKTOP_ACCEPTANCE PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
[ "$FAIL" -eq 0 ]
