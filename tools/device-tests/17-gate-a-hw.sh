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
ROOTFS_REL=${ROOTFS_REL:-files/rootfs-full}
LOADER_REL=${LOADER_REL:-files/linux/libld.so}
REPO_ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LOCATION_PROBE=${LOCATION_PROBE:-$REPO_ROOT/runtime/target/aarch64-unknown-linux-gnu/release/location-probe}
AUDIO_CAPTURE_PROBE=${AUDIO_CAPTURE_PROBE:-$REPO_ROOT/runtime/target/aarch64-unknown-linux-gnu/release/audio-capture-probe}
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

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
adb -s "$DEV" shell am start -W -n "$PKG/android.app.NativeActivity" >/dev/null 2>&1
sleep 2
read_husk_log() { adb_ 'logcat -d' | grep -E 'Husk *: ' || true; }
log=$(read_husk_log)
has() { printf '%s' "$log" | grep -qF "$1"; }
has "permissions|OK|all hardware permissions granted" \
    && ok "app reports all hardware permissions granted" \
    || bad "app never reported granted (is it running, and past the dialog?)"

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
frames=$(APP="$(adb_ "run-as $PKG sh -c 'cd /data/data/$PKG/files && mkdir -p camout && timeout 25 ./camera-client rootfs-full/run/husk/camera.sock camout 3'" 2>&1)" && printf '%s' "$APP")
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
