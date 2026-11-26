#!/bin/sh
# A-hw: the hardware permissions and the hardware they unlock, measured on an
# installation the *user* approved.
#
# This is the group round 4 adds next to gate A. A-core (14-gate-a-acceptance.sh)
# is unchanged: it measures the Linux userspace, and its numbers must not move
# because of a permission change. A-hw measures the other direction - that a
# plain `adb install` (no `-g`) ends with the app holding camera and microphone
# permission *because the user granted it*, and that the hardware behind those
# permissions is actually enumerated afterwards.
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
PASS=0; FAIL=0
ok()  { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
adb_() { adb -s "$DEV" shell "$@"; }

perm_line() { adb_ "dumpsys package $PKG" | grep -m1 "android.permission.$1: granted="; }

echo "== runtime permissions (granted by a user, not by install -g) =="
for perm in CAMERA RECORD_AUDIO; do
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
log=$(adb_ 'logcat -d' | grep -E 'Husk *: ' || true)
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
printf '%s' "$log" | grep -q 'camera|session|open|id' \
    && ok "app opened a Camera2 session" \
    || bad "no Camera2 session was opened"

has "caps|audio-in|present=yes" && ok "audio capture device present (enumeration only)" \
    || bad "audio capture device not present"

echo "== nothing died on the way =="
# A JNI misuse aborts the process (this is exactly how the
# 'illegal class name java/lang/String;' bug showed up), so a clean run must
# have no crash record for this package since the last install.
crash=$(adb -s "$DEV" logcat -d -b crash | grep -c "$PKG" || true)
[ "${crash:-0}" -eq 0 ] && ok "no crash records for $PKG" \
    || bad "crash buffer has $crash record(s) for $PKG"

echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
