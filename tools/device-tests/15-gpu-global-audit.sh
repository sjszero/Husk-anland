#!/bin/sh
# Check that a desktop session and a newly exec'd GTK application share the same
# hardware GPU runtime. This is a process-level audit, not a renderer claim:
# GSK_RENDERER is reported separately because GTK currently defaults to cairo.
set -u
P=0
F=0
S=0
fail() { F=$((F+1)); echo "  FAIL  $*"; }
ok()   { P=$((P+1)); echo "  PASS  $*"; }
skip() { S=$((S+1)); echo "  SKIP  $*"; }

pid_for() {
    ps -eo pid=,args= 2>/dev/null | grep -F "$1" | grep -v grep | \
        sed -n '1s/^ *\([0-9][0-9]*\).*/\1/p'
}

env_of() {
    tr '\000' '\n' < "/proc/$1/environ" 2>/dev/null || true
}

gpu_state() {
    pid=$1
    env=$(env_of "$pid")
    fds=$(ls -l "/proc/$pid/fd" 2>/dev/null || true)
    maps=$(grep -E '/(libvulkan_(panfrost|freedreno)[^/]*\.so)' "/proc/$pid/maps" 2>/dev/null || true)
    gsk=$(printf '%s\n' "$env" | sed -n 's/^GSK_RENDERER=//p' | head -1)
    [ -n "$gsk" ] || gsk=unset
    software=$(printf '%s\n' "$env" | grep -E '(^LIBGL_ALWAYS_SOFTWARE=1$|^GALLIUM_DRIVER=llvmpipe$|^MESA_LOADER_DRIVER_OVERRIDE=swrast$)' || true)
    if [ -n "$software" ]; then
        gpu=llvmpipe
    elif printf '%s\n' "$fds" | grep -qE '/dev/(mali0|kgsl-3d0)' && [ -n "$maps" ]; then
        # Loader defaults answer getenv() but are deliberately not written back to
        # /proc/PID/environ. Device fd + hardware Vulkan backend + no software
        # override is the process-level proof; do not require zink strings here.
        gpu=zink
    else
        gpu=unknown
    fi
    printf '%s %s\n' "$gpu" "$gsk"
}

check_pid() {
    label=$1
    pid=$2
    if [ -z "$pid" ] || [ ! -r "/proc/$pid/environ" ]; then
        fail "$label process not readable (pid=${pid:-missing})"
        return
    fi
    set -- $(gpu_state "$pid")
    gpu=${1:-unknown}
    gsk=${2:-unset}
    echo "  STATE $label pid=$pid gpu=$gpu gsk=$gsk"
    if [ "$gpu" = zink ]; then
        ok "$label uses zink + hardware Vulkan + device fd"
    else
        fail "$label did not expose the hardware GPU runtime (gpu=$gpu)"
    fi
}

echo '== existing desktop shell =='
shell_pid=$(pid_for 'usr/bin/gnome-shell')
check_pid gnome-shell "$shell_pid"

# The X client checks below are only meaningful against the session's own
# display. Inheriting it from the caller made the gate depend on how it was
# started: run through run-guest.sh without DISPLAY it failed with
# "Error: unable to open display" and reported 2 FAILs for a healthy session
# (2026-11-27). Read DISPLAY/XDG_RUNTIME_DIR from the shell's /proc environ and
# fall back to :0 only when there is no live session to ask.
if [ -n "$shell_pid" ]; then
    _senv=$(env_of "$shell_pid")
    for _k in DISPLAY XDG_RUNTIME_DIR; do
        eval "_cur=\${$_k:-}"
        [ -n "$_cur" ] && continue
        _v=$(printf '%s\n' "$_senv" | sed -n "s/^$_k=//p" | head -1)
        [ -n "$_v" ] || continue
        export "$_k=$_v"
    done
fi
: "${DISPLAY:=:0}"
export DISPLAY
echo "  STATE display=$DISPLAY runtime_dir=${XDG_RUNTIME_DIR:-unset}"

echo '== loader-default GL client =='
glx=$(timeout 20 /usr/bin/glxinfo -B 2>&1 || true)
if printf '%s\n' "$glx" | grep -q 'Device: zink Vulkan 1.3(Mali-G77'; then
    ok 'glxinfo saw zink Vulkan 1.3(Mali-G77)'
elif printf '%s\n' "$glx" | grep -q "couldn't find RGB GLX visual or fbconfig"; then
    skip 'this X server has no GLX visual (Xvfb fallback); EGL/GL loader defaults are covered by 18-gpu-loader-defaults.sh'
elif [ -z "$glx" ]; then
    fail 'glxinfo timed out or produced no output'
else
    fail 'glxinfo did not report zink Vulkan 1.3(Mali-G77)'
    printf '%s\n' "$glx" | sed -n '1,20p'
fi

echo '== nested application =='
pkill -x gnome-text-editor 2>/dev/null || true
pkill -f 'usr/bin/gnome-text-editor' 2>/dev/null || true
sleep 2
printf 'Husk GPU global audit\n' > /root/gpu-global-audit.txt
/usr/bin/gnome-text-editor /root/gpu-global-audit.txt >/tmp/gpu-global-audit.log 2>&1 &
app_pid=$!
sleep 8
check_pid nested-gtk "$app_pid"
if [ -n "${app_pid:-}" ]; then kill "$app_pid" 2>/dev/null || true; fi

echo "GPU_GLOBAL_AUDIT PASS=$P FAIL=$F SKIP=$S"
[ "$F" -eq 0 ]
