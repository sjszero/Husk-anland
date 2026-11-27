#!/bin/sh
# Guest-side standard audio path:
#   paplay -> PulseAudio module-pipe-sink -> Rust audio-relay -> app AAudio.
# This script is copied into the installed rootfs and executed through the Husk
# session; it does not touch the Android app process directly.
set -u
STATE=/tmp/husk-pulse
export PULSE_SERVER="unix:$STATE/native"
PULSE_PID=
RELAY_PID=
cleanup() {
    [ -n "${RELAY_PID:-}" ] && kill "$RELAY_PID" 2>/dev/null || true
    [ -n "${PULSE_PID:-}" ] && kill "$PULSE_PID" 2>/dev/null || true
    [ -n "${RELAY_PID:-}" ] && wait "$RELAY_PID" 2>/dev/null || true
    [ -n "${PULSE_PID:-}" ] && wait "$PULSE_PID" 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM

/usr/bin/pulseaudio --system=yes --daemonize=no --exit-idle-time=-1 -n \
    --file="$STATE/minimal.pa" >/tmp/husk-audio-pulse.log 2>&1 &
PULSE_PID=$!
i=0
while [ ! -S "$STATE/native" ] && [ "$i" -lt 50 ]; do
    sleep 0.1
    i=$((i + 1))
done
if [ ! -S "$STATE/native" ]; then
    echo "AUDIO_PIPELINE|FAIL|PulseAudio native socket missing"
    echo '---PULSE---'
    cat /tmp/husk-audio-pulse.log 2>/dev/null || true
    exit 1
fi
echo "AUDIO_PULSE_READY|pid=$PULSE_PID"

/usr/bin/audio-relay /run/husk/audio-out.sock "$STATE/audio.pcm" 8 \
    rate=48000 channels=2 >/tmp/husk-audio-relay.log 2>&1 &
RELAY_PID=$!
sleep 1
/usr/bin/paplay /usr/share/sounds/freedesktop/stereo/audio-test-signal.oga \
    >/tmp/husk-audio-paplay.log 2>&1
PAPLAY_RC=$?
wait "$RELAY_PID"
RELAY_RC=$?
RELAY_PID=
kill "$PULSE_PID" 2>/dev/null || true
wait "$PULSE_PID" 2>/dev/null || true
PULSE_PID=

echo "AUDIO_PLAYER_RC|paplay=$PAPLAY_RC|relay=$RELAY_RC"
echo '---PULSE---'
cat /tmp/husk-audio-pulse.log 2>/dev/null || true
echo '---RELAY---'
cat /tmp/husk-audio-relay.log 2>/dev/null || true
echo '---PAPLAY---'
cat /tmp/husk-audio-paplay.log 2>/dev/null || true
[ "$PAPLAY_RC" -eq 0 ] && [ "$RELAY_RC" -eq 0 ]
