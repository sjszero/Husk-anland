#!/bin/sh
# Run GNOME Snapshot through the standard desktop media path and report whether
# it survives the camera renegotiation window that used to end in SIGSEGV 139.
#
# This script is executed inside husk-session.sh with guest identity `husk`.
# Arguments: <output-prefix> [seconds]
set -u

base=${1:-/tmp/husk-snapshot-20s}
seconds=${2:-20}
case "$seconds" in
    ''|*[!0-9]*) echo "snapshot-20s-guest: invalid seconds: $seconds" >&2; exit 64 ;;
esac

export DISPLAY=:0
export XDG_RUNTIME_DIR=/tmp/runtime-husk
export DBUS_SESSION_BUS_ADDRESS=unix:abstract=linuxhub-session-bus-husk

rm -f "$base.log" "$base-gst.log" "$base-pipewire.log" "$base-v4l2.log" "$base.result"
v4l2_log=/tmp/husk-pipewire.log
v4l2_start=0
[ -r "$v4l2_log" ] && v4l2_start=$(wc -l <"$v4l2_log")

save_v4l2_delta() {
    end=0
    [ -r "$v4l2_log" ] && end=$(wc -l <"$v4l2_log")
    if [ "$end" -ge "$v4l2_start" ]; then
        tail -n "+$((v4l2_start + 1))" "$v4l2_log" >"$base-v4l2.log" 2>/dev/null || :
    else
        : >"$base-v4l2.log"
    fi
}

GST_DEBUG=pipewiresrc:7,videoflip:7 GST_DEBUG_FILE="$base-gst.log" GST_DEBUG_NO_COLOR=1 \
PIPEWIRE_DEBUG=2 PIPEWIRE_LOG="$base-pipewire.log" \
DORO_GPU_RENDERER=panvk /usr/local/bin/doro-gpu-run snapshot >"$base.log" 2>&1 &
pid=$!
printf 'pid=%s started_unix=%s seconds=%s\n' "$pid" "$(date -u +%s)" "$seconds" >"$base.result"

i=0
while [ "$i" -lt "$seconds" ]; do
    if ! kill -0 "$pid" 2>/dev/null; then
        break
    fi
    sleep 1
    i=$((i + 1))
done

if kill -0 "$pid" 2>/dev/null; then
    printf 'alive_after_%ss=true\n' "$seconds" >>"$base.result"
    kill -TERM "$pid" 2>/dev/null || true
    j=0
    while kill -0 "$pid" 2>/dev/null && [ "$j" -lt 50 ]; do
        sleep 0.1
        j=$((j + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
        printf 'termination=forced_kill\n' >>"$base.result"
        kill -KILL "$pid" 2>/dev/null || true
    else
        printf 'termination=clean_term\n' >>"$base.result"
    fi
    wait "$pid" 2>/dev/null
    rc=$?
    printf 'intentional_term_rc=%s\n' "$rc" >>"$base.result"
    save_v4l2_delta
    # A TERMed process normally exits 143. Any pre-existing crash is handled by
    # the branch below and must not be hidden by the intentional shutdown.
    exit 0
fi

wait "$pid"
rc=$?
printf 'alive_after_%ss=false\n' "$seconds" >>"$base.result"
printf 'exit=%s\n' "$rc" >>"$base.result"
if [ "$rc" -eq 139 ]; then
    printf 'verdict=crash_139\n' >>"$base.result"
else
    printf 'verdict=early_exit\n' >>"$base.result"
fi
save_v4l2_delta
exit "$rc"
