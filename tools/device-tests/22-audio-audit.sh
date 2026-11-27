#!/bin/sh
# Audio gate: provision the shipped Rust helpers, exercise the AAudio bridge
# directly, then play a standard guest sound through PulseAudio and the relay.
set -eu
DEV=${DEV:?usage: DEV=<adb serial> [ROOTFS=...] [LD=...] $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
PROFILE_DIR=${PROFILE_DIR:-$ROOT/runtime/target/aarch64-unknown-linux-gnu/release}
AUDIO_SETUP=${AUDIO_SETUP:-$PROFILE_DIR/audio-setup}
AUDIO_RELAY=${AUDIO_RELAY:-$PROFILE_DIR/audio-relay}
AUDIO_CAPTURE=${AUDIO_CAPTURE:-$PROFILE_DIR/audio-capture}
AUDIO_PROBE=${AUDIO_PROBE:-$PROFILE_DIR/audio-probe}
PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL + 1)); echo "  FAIL  $1"; }

device_path() {
    case "$1" in
        files/*) printf '/data/data/%s/%s\n' "$PKG" "$1" ;;
        *) printf '%s\n' "$1" ;;
    esac
}
ROOTFS=$(device_path "$ROOTFS")
LD=$(device_path "$LD")

adb_() { adb -s "$DEV" shell "$@"; }
adb_ input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
adb_ wm dismiss-keyguard >/dev/null 2>&1 || true
sleep 1

for f in "$AUDIO_SETUP" "$AUDIO_RELAY" "$AUDIO_CAPTURE" "$AUDIO_PROBE"; do
    if [ ! -x "$f" ]; then
        echo "audio-audit: missing $f (run tools/guest-config/build-guest-tools.sh)" >&2
        exit 69
    fi
done

for pair in \
    "$AUDIO_SETUP:/data/local/tmp/husk-audio-setup" \
    "$AUDIO_RELAY:/data/local/tmp/husk-audio-relay" \
    "$AUDIO_CAPTURE:/data/local/tmp/husk-audio-capture" \
    "$AUDIO_PROBE:/data/local/tmp/husk-audio-probe" \
    "$HERE/husk-audio-pipeline.sh:/data/local/tmp/husk-audio-pipeline.sh" \
    "$ROOT/tools/guest/husk-session.sh:$SESS"; do
    adb -s "$DEV" push "${pair%:*}" "${pair#*:}" >/dev/null
done

adb_ "run-as $PKG sh -c 'cp /data/local/tmp/husk-audio-setup $ROOTFS/usr/bin/audio-setup; cp /data/local/tmp/husk-audio-relay $ROOTFS/usr/bin/audio-relay; cp /data/local/tmp/husk-audio-capture $ROOTFS/usr/bin/audio-capture; cp /data/local/tmp/husk-audio-probe $ROOTFS/usr/bin/audio-probe; cp /data/local/tmp/husk-audio-pipeline.sh $ROOTFS/root/husk-audio-pipeline.sh; chmod 755 $ROOTFS/usr/bin/audio-setup $ROOTFS/usr/bin/audio-relay $ROOTFS/usr/bin/audio-capture $ROOTFS/usr/bin/audio-probe $ROOTFS/root/husk-audio-pipeline.sh'"

echo "== guest audio provisioning =="
setup_out=$(adb_ "run-as $PKG timeout 20 env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/audio-setup / --enable" 2>&1 || true)
printf '%s\n' "$setup_out" | sed -n 's/^audio-setup|PASS|/  NOTE  /p'
if printf '%s' "$setup_out" | grep -q '^audio-setup|PASS|'; then
    ok "audio-setup provisioned PulseAudio helper modules"
else
    bad "audio-setup failed: $(printf '%s' "$setup_out" | tail -3 | tr '\n' ' ')"
fi

i=0
while [ "$i" -lt 80 ]; do
    if adb_ "run-as $PKG test -S $ROOTFS/run/husk/audio-out.sock" >/dev/null 2>&1; then
        break
    fi
    sleep 0.25
    i=$((i + 1))
done
if adb_ "run-as $PKG test -S $ROOTFS/run/husk/audio-out.sock" >/dev/null 2>&1; then
    ok "app created audio-out.sock"
else
    bad "app did not create audio-out.sock after audio-setup"
fi

echo "== direct AAudio bridge =="
probe_out=$(adb_ "run-as $PKG timeout 15 env NR_LD=$LD sh $SESS $ROOTFS -- /usr/bin/audio-probe /run/husk/audio-out.sock 2" 2>&1 || true)
printf '%s\n' "$probe_out" | grep -E '^(DEVICES_BEGIN|DEVICES_END|DONE status=PASS|RESULT PASS)' | sed 's/^/  NOTE  /' || true
if printf '%s' "$probe_out" | grep -q '^RESULT PASS$' \
   && printf '%s' "$probe_out" | grep -q '^DONE status=PASS .*frames_written=96000'; then
    ok "AAudio enumerated devices, changed/restored volume, and wrote 96000 frames"
else
    bad "direct AAudio probe failed: $(printf '%s' "$probe_out" | tail -3 | tr '\n' ' ')"
fi

echo "== standard paplay -> PulseAudio -> Rust relay -> AAudio =="
pipeline_out=$(adb_ "run-as $PKG timeout 30 env NR_LD=$LD sh $SESS $ROOTFS -- /root/husk-audio-pipeline.sh" 2>&1 || true)
printf '%s\n' "$pipeline_out" | grep -E '^(AUDIO_PULSE_READY|AUDIO_PLAYER_RC|relay\|OPEN|relay\|CLOSE|relay\|PASS|RESULT PASS)' | sed 's/^/  NOTE  /' || true
if printf '%s' "$pipeline_out" | grep -q '^AUDIO_PLAYER_RC|paplay=0|relay=0$' \
   && printf '%s' "$pipeline_out" | grep -q '^relay|PASS|' \
   && printf '%s' "$pipeline_out" | grep -q '^RESULT PASS$'; then
    ok "standard paplay completed through PulseAudio and AAudio"
else
    bad "PulseAudio playback path failed: $(printf '%s' "$pipeline_out" | tail -6 | tr '\n' ' ')"
fi

echo "AUDIO_AUDIT PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
