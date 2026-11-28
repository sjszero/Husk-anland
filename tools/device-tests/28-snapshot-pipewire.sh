#!/bin/sh
# End-to-end GNOME Snapshot camera test for the Husk app.
#
# It deliberately uses the same guest user and media-session entry as the
# desktop. Launching Snapshot as guest root produces XDG_RUNTIME_DIR ownership
# errors and can take down PipeWire; that is a test-harness bug, not a camera
# result. Requires an already running GNOME X11 session on DISPLAY=:0.
set -eu

DEV=${1:?usage: $0 <adb-serial>}
PKG=${PKG:-com.linuxhub.husk}
APP=/data/user/0/$PKG
ROOTFS=${ROOTFS:-$APP/files/rootfs-full}
LD=${LD:-$APP/files/linux/husk/libld.so}
SESS=${SESS:-$APP/files/linux/husk/husk-session.sh}
MEDIA=${MEDIA:-/usr/local/bin/husk-media-session}
OUT=${OUT:-/tmp/husk-snapshot-pipewire}
SECONDS_TO_RUN=${SECONDS_TO_RUN:-20}
HERE=$(cd "$(dirname "$0")" && pwd)
GUEST_SCRIPT=$HERE/snapshot-20s-guest.sh
STEM=husk-snapshot-gate
STAGE=/data/local/tmp/$STEM-guest.sh
GUEST=/tmp/$STEM-guest.sh
PREFIX=/tmp/$STEM

[ -f "$GUEST_SCRIPT" ] || { echo "missing $GUEST_SCRIPT" >&2; exit 69; }
mkdir -p "$OUT"
adb -s "$DEV" push "$GUEST_SCRIPT" "$STAGE" >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'mkdir -p $ROOTFS/tmp && cp $STAGE $ROOTFS$GUEST && chmod 755 $ROOTFS$GUEST'"

run_husk() {
    adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS --user husk --cwd /home/husk -- usr/bin/env DISPLAY=:0 XDG_RUNTIME_DIR=/tmp/runtime-husk DBUS_SESSION_BUS_ADDRESS=unix:abstract=linuxhub-session-bus-husk $*"
}

ok=0
bad=0
pass() { ok=$((ok + 1)); printf 'PASS  %s\n' "$*"; }
fail() { bad=$((bad + 1)); printf 'FAIL  %s\n' "$*"; }

stop_log=$(run_husk "$MEDIA stop" 2>&1) || true
printf '%s\n' "$stop_log" >"$OUT/media-stop.log"
media=$(run_husk NR_V4L2_AUDIT=1 "$MEDIA start" 2>&1) || true
printf '%s\n' "$media" >"$OUT/media-start.log"
printf '%s\n' "$media"
printf '%s' "$media" | grep -q 'pipewire=ready' && pass 'PipeWire ready' || fail 'PipeWire not ready'
printf '%s' "$media" | grep -q 'wireplumber=ready' && pass 'WirePlumber ready' || fail 'WirePlumber not ready'

wp=$(run_husk /usr/bin/wpctl status 2>&1) || true
printf '%s\n' "$wp" >"$OUT/wpctl-status.log"
printf '%s' "$wp" | grep -q 'BACK.*\[v4l2\]' && pass 'BACK v4l2 device visible' || fail 'BACK v4l2 device missing'
printf '%s' "$wp" | grep -q 'FRONT.*\[v4l2\]' && pass 'FRONT v4l2 device visible' || fail 'FRONT v4l2 device missing'

rm -f "$OUT/snapshot.png" "$OUT/snapshot.log" "$OUT/snapshot-gst.log" "$OUT/snapshot-pipewire.log" "$OUT/snapshot-v4l2.log" "$OUT/snapshot.result"
adb -s "$DEV" shell "run-as $PKG sh -c 'NR_LD=$LD sh $SESS $ROOTFS --user husk --cwd /home/husk -- usr/bin/env DISPLAY=:0 XDG_RUNTIME_DIR=/tmp/runtime-husk DBUS_SESSION_BUS_ADDRESS=unix:abstract=linuxhub-session-bus-husk $GUEST $PREFIX $SECONDS_TO_RUN'" >"$OUT/snapshot-run.log" 2>&1 &
bridge=$!
sleep 8
adb -s "$DEV" exec-out screencap -p >"$OUT/snapshot.png" || true
rc=0
wait "$bridge" || rc=$?

copy_guest() {
    src_suffix=$1
    dst_name=$2
    adb -s "$DEV" exec-out run-as "$PKG" cat "$ROOTFS$PREFIX$src_suffix" >"$OUT/$dst_name" 2>"$OUT/$dst_name.readerr" || true
}
copy_guest .log snapshot.log
copy_guest -gst.log snapshot-gst.log
copy_guest -pipewire.log snapshot-pipewire.log
copy_guest -v4l2.log snapshot-v4l2.log
copy_guest .result snapshot.result

[ -s "$OUT/snapshot.result" ] || fail 'snapshot result missing'
grep -q "alive_after_${SECONDS_TO_RUN}s=true" "$OUT/snapshot.result" 2>/dev/null && pass "Snapshot alive after ${SECONDS_TO_RUN}s" || fail "Snapshot did not survive ${SECONDS_TO_RUN}s"
grep -q '^intentional_term_rc=143$' "$OUT/snapshot.result" 2>/dev/null && pass 'Snapshot terminated by intentional TERM (143)' || fail 'Snapshot did not stop via expected TERM'
grep -q 'verdict=crash_139' "$OUT/snapshot.result" 2>/dev/null && fail 'Snapshot crashed with 139' || pass 'No Snapshot 139'
grep -q 'stream is running and new caps are still acceptable; skipping renegotiation' "$OUT/snapshot-gst.log" 2>/dev/null && pass 'Redundant caps renegotiation skipped' || fail 'caps-subset fix log missing'
grep -Eq 'connect capture with path .*target-object [0-9]+' "$OUT/snapshot-gst.log" 2>/dev/null && pass 'pipewiresrc selected a real target object' || fail 'no numeric pipewiresrc target'
if grep -q 'connect capture with path .*target-object (null)' "$OUT/snapshot-gst.log" 2>/dev/null; then
    printf 'NOTE  initial null-target enumeration occurred before the numeric target\n'
fi
grep -q '\[v4l2\] DQBUF send FRAME' "$OUT/snapshot-v4l2.log" 2>/dev/null && pass 'Camera2 frames reached the V4L2 facade' || fail 'no V4L2 frame dequeue in Snapshot run'
grep -q '\[husk v4l2\] panic' "$OUT/snapshot-v4l2.log" 2>/dev/null && fail 'V4L2 bridge panicked' || pass 'No V4L2 panic in Snapshot run'
[ -s "$OUT/snapshot.png" ] && pass "screenshot captured: $OUT/snapshot.png" || fail 'screenshot missing'

printf 'RESULT PASS=%s FAIL=%s OUT=%s RUN_RC=%s\n' "$ok" "$bad" "$OUT" "$rc"
[ "$bad" -eq 0 ]
