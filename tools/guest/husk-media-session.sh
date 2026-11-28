#!/bin/sh
# Start/stop the per-session media stack used by standard desktop clients.
#
# GNOME Snapshot does not know about Husk's camera.sock. It enumerates cameras
# through GStreamer's pipewiredeviceprovider, so a working session needs
# PipeWire plus WirePlumber even when the Camera2 bridge itself is healthy.
#
# This helper is deliberately idempotent. The app may call it on every desktop
# launch, while a healthy reused session must not collect a second daemon.
set -eu

ACTION=${1:-start}
RT=${XDG_RUNTIME_DIR:?husk-media-session: XDG_RUNTIME_DIR is required}
STATE=$RT/husk-media
PIPE_PID=$STATE/pipewire.pid
WP_PID=$STATE/wireplumber.pid
PIPE_LOG=/tmp/husk-pipewire.log
WP_LOG=/tmp/husk-wireplumber.log
CONFIG=/etc/pipewire/pipewire.conf
WP_CONFIG=/etc/wireplumber/wireplumber.conf
WP_DATA_DIR=/usr/share/wireplumber
# The V4L2 facade is served by the App over camera.sock. Both PipeWire and
# WirePlumber dlopen spa.v4l2 at runtime, so the loader's startup GOT patch
# cannot cover either daemon. PipeWire creates/activates the V4L2 SPA device;
# WirePlumber owns the device node. Both processes therefore need the same
# preload. Pass the host-rooted path to the children only; a guest `/usr/...`
# path would be looked up by Android's dynamic linker before the loader starts.
if [ -n "${NR_ROOTFS:-}" ] && [ -f "$NR_ROOTFS/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so" ]; then
    V4L2_PRELOAD=$NR_ROOTFS/usr/lib/aarch64-linux-gnu/libhusk_v4l2.so
else
    V4L2_PRELOAD=
fi

mkdir -p "$STATE"
chmod 700 "$RT" "$STATE" 2>/dev/null || true

say() { printf '%s\n' "husk-media-session: $*"; }

pid_alive() {
    _pid_file=$1
    [ -s "$_pid_file" ] || return 1
    _pid=$(cat "$_pid_file" 2>/dev/null || true)
    [ -n "$_pid" ] || return 1
    kill -0 "$_pid" 2>/dev/null
}

probe_pipewire() {
    [ -S "$RT/pipewire-0" ] || return 1
    command -v pw-cli >/dev/null 2>&1 || return 0
    if command -v timeout >/dev/null 2>&1; then
        timeout 2 pw-cli info 0 >/dev/null 2>&1
    else
        pw-cli info 0 >/dev/null 2>&1
    fi
}

probe_wireplumber() {
    command -v wpctl >/dev/null 2>&1 || return 0
    if command -v timeout >/dev/null 2>&1; then
        timeout 2 wpctl status >/dev/null 2>&1
    else
        wpctl status >/dev/null 2>&1
    fi
}

stop_pid() {
    _pid_file=$1
    _name=$2
    pid_alive "$_pid_file" || return 0
    _pid=$(cat "$_pid_file" 2>/dev/null || true)
    kill -TERM "$_pid" 2>/dev/null || true
    _try=0
    while kill -0 "$_pid" 2>/dev/null && [ "$_try" -lt 20 ]; do
        _try=$((_try + 1))
        sleep 0.1
    done
    kill -KILL "$_pid" 2>/dev/null || true
    say "stopped $_name pid=$_pid"
}

start_pipewire() {
    if pid_alive "$PIPE_PID" && probe_pipewire; then
        say "reuse pipewire pid=$(cat "$PIPE_PID")"
        return 0
    fi

    stop_pid "$PIPE_PID" pipewire
    # A manually launched daemon has no pid file. Same-UID pkill is safe and
    # keeps a failed earlier launch from holding pipewire-0.lock.
    pkill -f -- '--argv0 pipewire' 2>/dev/null || true
    rm -f "$RT/pipewire-0" "$RT/pipewire-0.lock" \
          "$RT/pipewire-0-manager" "$RT/pipewire-0-manager.lock" \
          "$PIPE_PID" 2>/dev/null || true

    [ -r "$CONFIG" ] || { say "missing $CONFIG"; return 1; }
    : > "$PIPE_LOG" 2>/dev/null || true
    LD_PRELOAD="$V4L2_PRELOAD" \
    pipewire -c "$CONFIG" >"$PIPE_LOG" 2>&1 &
    _pid=$!
    printf '%s\n' "$_pid" > "$PIPE_PID"

    _try=0
    while [ "$_try" -lt 50 ]; do
        if probe_pipewire; then
            say "started pipewire pid=$_pid"
            return 0
        fi
        kill -0 "$_pid" 2>/dev/null || break
        _try=$((_try + 1))
        sleep 0.1
    done
    say "pipewire failed to become ready; log=$PIPE_LOG"
    return 1
}

start_wireplumber() {
    if pid_alive "$WP_PID" && probe_wireplumber; then
        say "reuse wireplumber pid=$(cat "$WP_PID")"
        return 0
    fi

    stop_pid "$WP_PID" wireplumber
    pkill -f -- '--argv0 wireplumber' 2>/dev/null || true
    rm -f "$WP_PID" 2>/dev/null || true

    [ -r "$WP_CONFIG" ] || { say "missing $WP_CONFIG"; return 1; }
    if [ -z "$V4L2_PRELOAD" ]; then
        say "missing libhusk_v4l2.so; standard camera clients will not see /dev/video*"
        return 1
    fi
    : > "$WP_LOG" 2>/dev/null || true
    LD_PRELOAD="$V4L2_PRELOAD" \
    WIREPLUMBER_DATA_DIR="$WP_DATA_DIR" \
    wireplumber -c "$WP_CONFIG" >"$WP_LOG" 2>&1 &
    _pid=$!
    printf '%s\n' "$_pid" > "$WP_PID"

    _try=0
    while [ "$_try" -lt 50 ]; do
        if kill -0 "$_pid" 2>/dev/null && probe_wireplumber; then
            say "started wireplumber pid=$_pid"
            return 0
        fi
        kill -0 "$_pid" 2>/dev/null || break
        _try=$((_try + 1))
        sleep 0.1
    done
    say "wireplumber failed to become ready; log=$WP_LOG"
    return 1
}

stop_all() {
    stop_pid "$WP_PID" wireplumber
    stop_pid "$PIPE_PID" pipewire
    # Cover manually started or pid-file-less copies from earlier test rounds.
    pkill -f -- '--argv0 wireplumber' 2>/dev/null || true
    pkill -f -- '--argv0 pipewire' 2>/dev/null || true
    rm -f "$WP_PID" "$PIPE_PID" "$RT/pipewire-0" "$RT/pipewire-0.lock" \
          "$RT/pipewire-0-manager" "$RT/pipewire-0-manager.lock" 2>/dev/null || true
}

status() {
    if pid_alive "$PIPE_PID" && probe_pipewire; then
        say "pipewire=ready pid=$(cat "$PIPE_PID")"
    else
        say "pipewire=not-ready"
    fi
    if pid_alive "$WP_PID" && probe_wireplumber; then
        say "wireplumber=ready pid=$(cat "$WP_PID")"
    else
        say "wireplumber=not-ready"
    fi
}

case "$ACTION" in
    start)
        start_pipewire
        start_wireplumber
        status
        ;;
    stop)
        stop_all
        ;;
    status)
        status
        ;;
    *)
        echo "usage: husk-media-session.sh [start|stop|status]" >&2
        exit 64
        ;;
esac
