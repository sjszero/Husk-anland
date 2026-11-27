#!/bin/sh
# Verify that final-symlink replacement cannot redirect chmod/utimensat/truncate
# from the guest root to an App-private host file outside the rootfs.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO_ROOT=$(cd "$HERE/../.." && pwd)
DEV=${DEV:?usage: DEV=<serial> sh run-path-race-probe.sh}
PKG=${PKG:-com.linuxhub.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
RUNTIME_PROBE=${RUNTIME_PROBE:-$REPO_ROOT/runtime/target/aarch64-unknown-linux-gnu/release/runtime-probe}

# The Makefile passes workspace-relative paths. They are host-side names and
# must be normalized before crossing into the guest; passing "files/linux/..."
# straight to husk-session makes it look for a relative loader on the device.
device_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        files/linux/*) printf '%s/%s\n' "$APPDIR" "${1#files/linux/}" ;;
        files/*) printf '/data/data/%s/%s\n' "$PKG" "$1" ;;
        *) printf '%s\n' "$1" ;;
    esac
}
ROOTFS=$(device_path "$ROOTFS")
LD=$(device_path "$LD")

[ -x "$RUNTIME_PROBE" ] || {
    echo "--- path-race-probe: SKIP (missing $RUNTIME_PROBE; run tools/guest-config/build-guest-tools.sh)" >&2
    exit 0
}
APP=$(adb -s "$DEV" shell "run-as $PKG pwd" | tr -d '\r')
SENTINEL=$APP/files/husk-path-race-sentinel
cleanup() {
    adb -s "$DEV" shell "run-as $PKG rm -f '$SENTINEL' '$ROOTFS/root/runtime-probe'" >/dev/null 2>&1 || true
    adb -s "$DEV" shell 'rm -f /data/local/tmp/husk-runtime-probe' >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

adb -s "$DEV" shell "run-as $PKG sh -c 'printf HOST_SENTINEL_DO_NOT_TOUCH > \"$SENTINEL\"; chmod 640 \"$SENTINEL\"; touch -t 202001010101 \"$SENTINEL\"'"
before_meta=$(adb -s "$DEV" shell "run-as $PKG stat -c '%a:%s:%Y' '$SENTINEL'" | tr -d '\r')
before_hash=$(adb -s "$DEV" shell "run-as $PKG sha256sum '$SENTINEL'" | awk '{print $1}')

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" "$SESS" >/dev/null
adb -s "$DEV" push "$RUNTIME_PROBE" /data/local/tmp/husk-runtime-probe >/dev/null
adb -s "$DEV" shell "run-as $PKG sh -c 'cp /data/local/tmp/husk-runtime-probe $ROOTFS/root/runtime-probe && chmod 755 $ROOTFS/root/runtime-probe'"

if ! out=$(adb -s "$DEV" shell "run-as $PKG env NR_LD=$LD NR_FAKE_ROOT=0 sh $SESS $ROOTFS -- /root/runtime-probe path-race '$SENTINEL'" 2>&1); then
    printf '%s\n' "$out" >&2
    echo '--- path-race-probe: execution failed' >&2
    exit 1
fi
printf '%s\n' "$out"

printf '%s\n' "$out" | grep -q 'PASS host-sentinel-not-visible' \
    || { echo "FAIL guest could see the host sentinel path" >&2; exit 1; }
printf '%s\n' "$out" | grep -q '^PATH_RACE_DONE$' \
    || { echo "FAIL race probe did not finish" >&2; exit 1; }

after_meta=$(adb -s "$DEV" shell "run-as $PKG stat -c '%a:%s:%Y' '$SENTINEL'" | tr -d '\r')
after_hash=$(adb -s "$DEV" shell "run-as $PKG sha256sum '$SENTINEL'" | awk '{print $1}')
if [ "$before_meta" != "$after_meta" ] || [ "$before_hash" != "$after_hash" ]; then
    echo "FAIL host sentinel changed: before=$before_meta/$before_hash after=$after_meta/$after_hash" >&2
    exit 1
fi

echo "PASS host sentinel unchanged: $after_meta/$after_hash"
