#!/bin/sh
# A-hw: the hardware permissions and the hardware they unlock, measured on an
# installation the *user* approved.
#
# This is the group round 4 adds next to gate A. A-core (14-gate-a-acceptance.sh)
# is unchanged: it measures the Linux userspace, and its numbers must not move
# because of a permission change. A-hw measures the other direction - that a
# plain `adb install` (no `-g`) ends with the app holding camera, microphone and
# coarse-location permission *because the user granted them*, and that the
# hardware behind those permissions is actually enumerated afterwards.
#
#   DEV=UKHMMB8PYTGECMNV ./17-gate-a-hw.sh
#
# Prerequisites, all of them real user actions:
#   1. `adb install` the APK **without** `-g`;
#   2. launch it and answer the system dialog (deny twice to reach the settings
#      page, then allow there - that is the path this gate is paired with);
#   3. leave the app running so its startup lines are in logcat.
set -u
DEV=${DEV:?usage: DEV=<serial> $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS_REL=${ROOTFS_REL:-${ROOTFS:-files/rootfs-full}}   # ROOTFS is the name every other gate uses
LOADER_REL=${LOADER_REL:-files/linux/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
REPO_ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SENSOR_PROBE=${SENSOR_PROBE:-$REPO_ROOT/tools/device-tests/husk-sensor-probe.sh}
LOCATION_PROBE=${LOCATION_PROBE:-$REPO_ROOT/runtime/target/aarch64-unknown-linux-gnu/release/location-probe}
AUDIO_CAPTURE_PROBE=${AUDIO_CAPTURE_PROBE:-$REPO_ROOT/runtime/target/aarch64-unknown-linux-gnu/release/audio-capture-probe}
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

# OxygenOS/ColorOS freezes the app in the `LcdOff` state. A-hw starts by
# reading the app's hardware bridges, so leaving the phone asleep turns a
# healthy camera/audio bridge into an empty socket and a false FAIL. Wake and
# dismiss the keyguard before touching app state; the test itself does not
# change permissions or grant hardware access.
adb_ input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
adb_ wm dismiss-keyguard >/dev/null 2>&1 || true
sleep 1

perm_line() { adb_ "dumpsys package $PKG" | grep -m1 "android.permission.$1: granted="; }

echo "== runtime permissions (granted by a user, not by install -g) =="
for perm in CAMERA RECORD_AUDIO ACCESS_COARSE_LOCATION; do
    line=$(perm_line "$perm")
    case "$line" in
        *"granted=true"*) ok "$perm granted=true" ;;
        *) bad "$perm not granted (got: ${line:-<absent>})" ;;
    esac
    # USER_FIXED means "denied, don't ask again": the dialog will never come back
    # and the settings page is the only way in. A passed gate must not be sitting
    # in that state.
    case "$line" in
        *USER_FIXED*) bad "$perm carries USER_FIXED (denied-and-fixed)" ;;
        *) ok "$perm is not USER_FIXED" ;;
    esac
done

echo "== the app read that state back =="
# logcat is a ring buffer. Force a fresh onResume and read the permission
# state it reports now, instead of accepting an old line that may have rolled
# out while the app was running. Only the main buffer is cleared; the crash
# buffer is checked later and must stay intact.
adb_ 'logcat -b main -c'
adb_ 'input keyevent KEYCODE_HOME'
sleep 1
# The husk-app host uses android.app.NativeActivity, but the .husk build of the
# LinuxHub app uses its own MainActivity. Resolve the launcher instead of
# hard-coding one host's activity name (2026-11-28: `am start` silently failed
# on the LinuxHub host, so the app stayed backgrounded and the Camera2 open was
# refused with ACAMERA_ERROR_CAMERA_DISABLED).
launcher=$(adb_ "cmd package resolve-activity --brief $PKG" 2>/dev/null | tail -1 | tr -d '\r')
[ -n "$launcher" ] && adb -s "$DEV" shell am start -W -n "$launcher" >/dev/null 2>&1
sleep 2
read_husk_log() { adb_ 'logcat -d' | grep -E 'Husk *: ' || true; }
log=$(read_husk_log)
has() { printf '%s' "$log" | grep -qF "$1"; }
if has "permissions|OK|all hardware permissions granted"; then
    ok "app reports all hardware permissions granted"
elif adb_ "dumpsys package $PKG" 2>/dev/null | sed -n '/runtime permissions/,/^$/p' | grep -E 'permission\.(CAMERA|RECORD_AUDIO|ACCESS_COARSE_LOCATION): granted=true' | grep -q 'granted=true'; then
    # Hosts that do not log the husk-app permission line (the LinuxHub .husk
    # build) still have to prove the state: read it from the platform.
    n_granted=$(adb_ "dumpsys package $PKG" 2>/dev/null | sed -n '/runtime permissions/,/^$/p' | grep -cE 'permission\.(CAMERA|RECORD_AUDIO|ACCESS_COARSE_LOCATION): granted=true')
    ok "platform reports the hardware permissions granted ($n_granted/3; host does not log the husk-app line)"
else
    bad "app never reported granted (is it running, and past the dialog?)"
fi

echo "== the hardware behind the permission actually delivers =="
# A real capture, end to end: the client opens the app's socket, the app opens a
# Camera2 session, frames come back. This is the part the permission gates -
# `CameraManager.openCamera` throws SecurityException without it - so a PASS here
# is evidence, not an enumeration.
#
# NOTE (measured 2026-11-26): `camera|list|count` and `camera|v4l2|tree|devices`
# appear even while the permission is denied, because listing cameras and
# publishing /dev/videoN nodes do not need it. They are printed below for
# context and are deliberately *not* counted as PASS lines.
printf '%s' "$log" | sed -n 's/.*\(camera|list|count=[0-9]*\).*/  NOTE  \1 (not permission-gated)/p' | tail -1
printf '%s' "$log" | sed -n 's/.*\(camera|v4l2|tree|devices=[0-9]*\).*/  NOTE  \1 (not permission-gated)/p' | tail -1
# `cd .../files` + a ROOTFS_REL that already starts with `files/` produced
# `files/files/linux/...` and the client failed with an empty error (2026-11-28).
# Stay in the app data dir so ROOTFS_REL is used exactly as the caller passed it.
frames=$(APP="$(adb_ "run-as $PKG sh -c 'cd /data/data/$PKG && mkdir -p files/camout && timeout 25 ./files/camera-client $ROOTFS_REL/run/husk/camera.sock files/camout 3'" 2>&1)" && printf '%s' "$APP")
n=$(printf '%s' "$frames" | grep -c 'camera-client|FRAME' || true)
if printf '%s' "$frames" | grep -q 'camera-client|PASS'; then
    ok "captured $n frame(s) through Camera2 ($(printf '%s' "$frames" | sed -n 's/.*1280x720.*bytes=\([0-9]*\)|min.*/\1/p' | head -1) bytes each)"
else
    bad "capture failed: $(printf '%s' "$frames" | head -2 | tr '\n' ' ')"
fi
# The client's capture creates the session log line during this check, after
# the permission log snapshot. Refresh before asserting it instead of looking
# at a stale buffer.
log=$(read_husk_log)
has "camera|session|open|id" && ok "app opened a Camera2 session"     || bad "no Camera2 session was opened"

# The capability matrix is durable; logcat is a ring buffer and can roll over
# while the camera test runs. Read the app's canonical matrix instead of
# assuming the startup lines are still present in logcat.
read_caps() {
    adb_ "run-as $PKG cat files/husk-capabilities.txt" 2>/dev/null || true
}
caps=$(read_caps)
caps_has() { printf '%s' "$caps" | grep -qF "$1"; }
case "$caps" in
    '# husk-capabilities v1'*) ok "capability matrix readable" ;;
    *) bad "capability matrix missing or unreadable" ;;
esac
caps_has "caps|audio-in|present=yes" && ok "audio capture device present" \
    || bad "audio capture device not present"

# A matrix entry is not a sensor stream. Read all five default classes through
# the three standard IIO views and require continuous classes to keep updating.
echo "== five default sensors are readable through Linux IIO =="
sensor_probe=/data/local/tmp/husk-sensor-probe
sensor_out=
if adb -s "$DEV" push "$SENSOR_PROBE" "$sensor_probe" >/dev/null 2>&1 \
   && adb -s "$DEV" push "$REPO_ROOT/tools/guest/husk-session.sh" "$SESS" >/dev/null 2>&1; then
    case "$ROOTFS_REL" in
        /*) sensor_rootfs=$ROOTFS_REL ;;
        *) sensor_rootfs=/data/data/$PKG/$ROOTFS_REL ;;
    esac
    case "$LOADER_REL" in
        /*) sensor_loader=$LOADER_REL ;;
        *) sensor_loader=/data/data/$PKG/$LOADER_REL ;;
    esac
    adb_ "run-as $PKG sh -c 'cp $sensor_probe $sensor_rootfs/root/husk-sensor-probe && chmod 755 $sensor_rootfs/root/husk-sensor-probe'" >/dev/null 2>&1 || true
    sensor_out=$(adb_ "run-as $PKG env NR_LD=$sensor_loader sh $SESS $sensor_rootfs -- /root/husk-sensor-probe" 2>&1 || true)
    printf '%s\n' "$sensor_out" | sed -n 's/^SENSOR_PATH|/  NOTE  /p' | head -15
else
    sensor_out='SENSOR_AUDIT|FAIL|could not push sensor probe or session script'
fi
if printf '%s' "$sensor_out" | grep -q '^SENSOR_AUDIT|PASS|classes=5|paths=3|stream=live$'; then
    ok "five default sensors readable through three IIO views"
else
    bad "sensor audit failed: $(printf '%s' "$sensor_out" | tail -4 | tr '\n' ' ')"
fi

# Enumeration is not capture. Feed the app's own AAudio bridge through the
# guest-side socket and require real PCM bytes with at least one non-silent
# sample. The marker only enables the already-implemented bridge; it does not
# grant the microphone permission. The probe is copied to the private rootfs
# because `run-as` cannot execute a host file from /data/local/tmp.
echo "== authorized microphone delivers PCM =="
probe=/data/local/tmp/husk-audio-capture-probe
if [ ! -x "$AUDIO_CAPTURE_PROBE" ]; then
    bad "missing $AUDIO_CAPTURE_PROBE (run tools/guest-config/build-guest-tools.sh)"
    AUDIO_CAPTURE_PROBE=
elif adb -s "$DEV" push "$AUDIO_CAPTURE_PROBE" "$probe" >/dev/null 2>&1; then
    adb_ "run-as $PKG sh -c 'cp $probe $ROOTFS_REL/root/audio-capture-probe && chmod 755 $ROOTFS_REL/root/audio-capture-probe && mkdir -p $ROOTFS_REL/tmp/husk-pulse && : > $ROOTFS_REL/tmp/husk-pulse/enabled-in'" >/dev/null 2>&1 || true
else
    bad "could not push audio capture probe"
    AUDIO_CAPTURE_PROBE=
fi
sleep 2
audio=$(
  if [ -n "$AUDIO_CAPTURE_PROBE" ]; then
    adb_ "run-as $PKG sh -c 'R=\$PWD/$ROOTFS_REL; L=\$PWD/$LOADER_REL; cd \"\$R\" && NR_ROOTFS=\"\$R\" NR_LD=\"\$L\" HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \"\$L\" --library-path \"\$R/lib/aarch64-linux-gnu:\$R/usr/lib/aarch64-linux-gnu\" ./root/audio-capture-probe /run/husk/audio-in.sock 1'" 2>&1 || true
  fi
)
printf '%s\n' "$audio" | grep -E '^capture-probe\|PASS|^CAPTURE_OK' | sed 's/^/  NOTE  /' || true
if printf '%s' "$audio" | grep -q '^capture-probe|PASS|'; then
    bytes=$(printf '%s' "$audio" | sed -n 's/^capture-probe|PASS|.*bytes=\([0-9]*\).*/\1/p' | tail -1)
    peak=$(printf '%s' "$audio" | sed -n 's/^capture-probe|PASS|.*peak=\([0-9]*\).*/\1/p' | tail -1)
    if [ "${bytes:-0}" -gt 0 ] && [ "${peak:-0}" -gt 0 ]; then
        ok "microphone returned ${bytes} PCM bytes (peak=${peak})"
    else
        bad "microphone bridge returned no non-silent PCM (bytes=${bytes:-0} peak=${peak:-0})"
    fi
else
    bad "microphone capture probe failed: $(printf '%s' "$audio" | tail -2 | tr '\n' ' ')"
fi

echo "== authorized coarse location delivers gpsd TPV =="
locprobe=/data/local/tmp/husk-location-probe
if [ ! -x "$LOCATION_PROBE" ]; then
    bad "missing $LOCATION_PROBE (run tools/guest-config/build-guest-tools.sh)"
    LOCATION_PROBE=
elif adb -s "$DEV" push "$LOCATION_PROBE" "$locprobe" >/dev/null 2>&1; then
    adb_ "run-as $PKG sh -c 'cp $locprobe $ROOTFS_REL/root/location-probe && chmod 755 $ROOTFS_REL/root/location-probe'" >/dev/null 2>&1 || true
else
    bad "could not push location probe"
    LOCATION_PROBE=
fi
location=$(
  if [ -n "$LOCATION_PROBE" ]; then
    adb_ "run-as $PKG sh -c 'R=\$PWD/$ROOTFS_REL; L=\$PWD/$LOADER_REL; cd \"\$R\" && NR_ROOTFS=\"\$R\" NR_LD=\"\$L\" HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \"\$L\" --library-path \"\$R/lib/aarch64-linux-gnu:\$R/usr/lib/aarch64-linux-gnu\" ./root/location-probe'" 2>&1 || true
  fi
)
if printf '%s' "$location" | grep -q '^location-probe|PASS|'; then
    ok "gpsd delivered a last-known fix ($(printf '%s' "$location" | sed -n 's/^location-probe|PASS|//p'))"
else
    bad "location probe failed: $(printf '%s' "$location" | tail -2 | tr '\n' ' ')"
fi

echo "== nothing died on the way =="
# A JNI misuse aborts the process (this is exactly how the
# 'illegal class name java/lang/String;' bug showed up), so a clean run must
# have no crash record for this package since the last install.
# `camera-client` is a deliberately separate test process. Its historical
# SIGABRT must not be attributed to the app; count crash records whose Cmdline
# is the app process itself.
crash=$(adb -s "$DEV" logcat -d -b crash | grep -cE 'Cmdline: .*com\.linuxhub\.husk' || true)
[ "${crash:-0}" -eq 0 ] && ok "no crash records for the app process"     || bad "crash buffer has $crash app-process record(s) for $PKG"

echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
