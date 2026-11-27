#!/bin/sh
# 23 · Desktop multi-application GPU/process gate.
#
# Requires 22-desktop-acceptance.sh to have left a ready embedded X11 desktop.
# It launches three GUI processes through the real loader, proves each retained
# its own PID, hardware Vulkan/device-fd state and DISPLAY/session-bus identity,
# keeps them concurrent for RUN_SECS, then captures another frame. It never
# restarts or kills another package and removes only the applications it started.
#
#   DEV=<serial> [RUN_SECS=60] ./23-desktop-multiapp.sh
#
# Exit: 0 PASS, 1 test failure, 2 device-health/prerequisite blocked.
set -u
DEV=${DEV:?usage: DEV=<adb serial> $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
RUN_SECS=${RUN_SECS:-60}
MAX_LOAD=${MAX_LOAD:-12}
MAX_THREADS=${MAX_THREADS:-12000}
MIN_MEM_KB=${MIN_MEM_KB:-2097152}
HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
HUSK_TOOLS=${HUSK_TOOLS:-$REPO_ROOT/runtime/target/debug/husk-tools}
PASS=0; FAIL=0; SKIP=0
TMP=${TMPDIR:-/tmp}/husk-desktop-multiapp-$$
mkdir -p "$TMP" || exit 2
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
ok()   { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
skip() { SKIP=$((SKIP+1)); echo "  SKIP  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

echo "== prerequisites =="
[ "$(adb -s "$DEV" get-state 2>/dev/null || true)" = device ] || { bad "adb device offline"; exit 2; }
[ -x "$HUSK_TOOLS" ] || { bad "missing $HUSK_TOOLS"; exit 2; }
[ "$RUN_SECS" -ge 1 ] 2>/dev/null || { bad "RUN_SECS must be >= 1"; exit 2; }
load1=$(adb_ 'cat /proc/loadavg' 2>/dev/null | awk '{print $1}')
threads=$(adb_ 'ps -e -T 2>/dev/null | wc -l' 2>/dev/null | tr -d ' \r')
mem_kb=$(adb_ 'grep ^MemAvailable: /proc/meminfo' 2>/dev/null | awk '{print $2}')
case "$load1" in ''|*[!0-9.]*) load1=999 ;; esac
case "$threads" in ''|*[!0-9]*) threads=999999 ;; esac
case "$mem_kb" in ''|*[!0-9]*) mem_kb=0 ;; esac
echo "  STATE load1=$load1 threads=$threads MemAvailable=${mem_kb}kB"
awk -v n="$load1" -v max="$MAX_LOAD" 'BEGIN { exit !(n <= max) }' \
    && ok "load1 below ceiling" || bad "load1=$load1 exceeds $MAX_LOAD"
[ "$threads" -le "$MAX_THREADS" ] \
    && ok "process/threads below ceiling" || bad "threads=$threads exceeds $MAX_THREADS"
[ "$mem_kb" -ge "$MIN_MEM_KB" ] \
    && ok "MemAvailable above floor" || bad "MemAvailable=${mem_kb}kB below $MIN_MEM_KB kB"
[ "$FAIL" -eq 0 ] || { echo "DESKTOP_MULTIAPP PASS=$PASS FAIL=$FAIL SKIP=$SKIP (BLOCKED: device health)"; exit 2; }

status=$(adb_ "run-as $PKG cat $ROOTFS/tmp/husk-desktop.status" 2>/dev/null || true)
if [ "$(printf '%s\n' "$status" | sed -n '1p')" = ready ]; then
    ok "desktop readiness is ready"
else
    bad "desktop is not ready; run make desktop-acceptance first"
    exit 2
fi
if adb_ "run-as $PKG sh -c 'cat > $ROOTFS/tmp/husk-exec.sh'" < "$HERE/husk-exec.sh" &&
   adb_ "run-as $PKG chmod 755 $ROOTFS/tmp/husk-exec.sh" >/dev/null; then
    ok "guest exec helper staged"
else
    bad "could not stage guest exec helper"
    exit 2
fi

launch() {
    tag=$1
    shift
    command="$*"
    pid=$(adb_ "run-as $PKG sh -c 'cd $ROOTFS && export DISPLAY=:0 XDG_RUNTIME_DIR=/run/rt DBUS_SESSION_BUS_ADDRESS=unix:path=/run/rt/bus LANG=zh_CN.UTF-8 LANGUAGE=zh_CN:zh && sh tmp/husk-exec.sh $ROOTFS $LD $command >tmp/husk-multiapp-$tag.log 2>&1 </dev/null & echo \$!'" 2>/dev/null | tr -d ' \r')
    case "$pid" in ''|*[!0-9]*) pid= ;; esac
    printf '%s\n' "$pid"
}

printf 'Husk desktop multi-app soak\n' > "$TMP/editor.txt"
adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $ROOTFS/root/husk-multiapp.txt'" < "$TMP/editor.txt" || true
editor_pid=$(launch editor usr/bin/gnome-text-editor /root/husk-multiapp.txt)
calc_pid=$(launch calculator usr/bin/gnome-calculator)
files_pid=$(launch files usr/bin/nautilus --no-desktop)
printf '%s\n' "$editor_pid $calc_pid $files_pid" | sed 's/^/  STATE launched pids: /'
[ -n "$editor_pid" ] && ok "gnome-text-editor launch returned a pid" || bad "gnome-text-editor did not launch"
[ -n "$calc_pid" ] && ok "gnome-calculator launch returned a pid" || bad "gnome-calculator did not launch"
[ -n "$files_pid" ] && ok "nautilus launch returned a pid" || bad "nautilus did not launch"
sleep 8

# Single-instance GUI apps may hand the window to an existing process and let
# the launcher pid exit. Resolve the live process by its real command line after
# launch, then audit that pid instead of trusting a transient wrapper pid.
find_pid() {
    needle=$1
    line=$(adb_ "run-as $PKG sh -c 'ps -A -o PID,ARGS 2>/dev/null | grep -F $needle | grep -v grep'" 2>/dev/null | head -1)
    printf '%s\n' "$line" | awk '{print $1}'
}
editor_actual=$(find_pid gnome-text-editor)
calc_actual=$(find_pid gnome-calculator)
files_actual=$(find_pid nautilus)
[ -z "$editor_actual" ] || editor_pid=$editor_actual
[ -z "$calc_actual" ] || calc_pid=$calc_actual
[ -z "$files_actual" ] || files_pid=$files_actual
printf '%s\n' "$editor_pid $calc_pid $files_pid" | sed 's/^/  STATE live pids: /'

check_app() {
    label=$1; pid=$2; needle=$3
    if [ -z "$pid" ] || ! adb_ "run-as $PKG kill -0 $pid" >/dev/null 2>&1; then
        bad "$label is not alive (pid=${pid:-missing})"
        return
    fi
    argv=$(adb_ "run-as $PKG cat /proc/$pid/cmdline" 2>/dev/null | tr '\000' ' ')
    case "$argv" in *"$needle"*) ok "$label process is alive with expected executable" ;; *) bad "$label argv mismatch ($argv)" ;; esac
    env=$(adb_ "run-as $PKG cat /proc/$pid/environ" 2>/dev/null | tr '\000' '\n' || true)
    printf '%s\n' "$env" | grep -qx 'DISPLAY=:0' \
        && ok "$label inherited DISPLAY=:0" || bad "$label has no DISPLAY=:0"
    printf '%s\n' "$env" | grep -qx 'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/rt/bus' \
        && ok "$label inherited the desktop session bus" || bad "$label has no desktop session bus"
    fds=$(adb_ "run-as $PKG ls -l /proc/$pid/fd" 2>/dev/null || true)
    maps=$(adb_ "run-as $PKG grep -E '/libvulkan_(panfrost|freedreno)' /proc/$pid/maps" 2>/dev/null || true)
    printf '%s\n' "$fds" | grep -qE '/dev/(mali0|kgsl-3d0)' \
        && ok "$label holds a hardware GPU device fd" || bad "$label holds no Mali/KGSL fd"
    [ -n "$maps" ] \
        && ok "$label loaded a hardware Vulkan backend" || bad "$label loaded no hardware Vulkan backend"
    printf '%s\n' "$env" | grep -qE '(^LIBGL_ALWAYS_SOFTWARE=1$|^GALLIUM_DRIVER=llvmpipe$|^MESA_LOADER_DRIVER_OVERRIDE=swrast$)' \
        && bad "$label has a software-render override" || ok "$label has no software-render override"
}

check_app editor "$editor_pid" gnome-text-editor
check_app calculator "$calc_pid" gnome-calculator
check_app files "$files_pid" nautilus

echo "== concurrent soak ${RUN_SECS}s =="
sleep "$RUN_SECS"
for pair in "editor:$editor_pid" "calculator:$calc_pid" "files:$files_pid"; do
    label=${pair%%:*}; pid=${pair#*:}
    if [ -n "$pid" ] && adb_ "run-as $PKG kill -0 $pid" >/dev/null 2>&1; then
        ok "$label survived ${RUN_SECS}s"
    else
        bad "$label exited during soak"
    fi
done

if adb -s "$DEV" exec-out screencap > "$TMP/multiapp.raw" 2>"$TMP/screencap.err"; then
    if "$HUSK_TOOLS" screenshot-audit --strict "$TMP/multiapp.raw"; then
        ok "multi-app frame is not all-black/large-missing"
    else
        bad "multi-app frame looks black or has a large missing region"
    fi
else
    bad "multi-app screencap failed: $(tr '\n' ' ' < "$TMP/screencap.err" | head -c 240)"
fi

# Remove only processes launched by this gate.
for pid in "$editor_pid" "$calc_pid" "$files_pid"; do
    [ -n "$pid" ] && adb_ "run-as $PKG kill $pid" >/dev/null 2>&1 || true
done

echo "DESKTOP_MULTIAPP PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
[ "$FAIL" -eq 0 ]
