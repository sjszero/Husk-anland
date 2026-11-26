#!/bin/sh
# Verify that final-symlink replacement cannot redirect chmod/utimensat/truncate
# from the guest root to an App-private host file outside the rootfs.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
DEV=${DEV:?usage: DEV=<serial> sh run-path-race-probe.sh}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/linux/distributions/ubuntu-24.04/rootfs}
LD=${LD:-files/linux/husk/libld.so}

APP=$(adb -s "$DEV" shell "run-as $PKG pwd" | tr -d '\r')
SENTINEL=$APP/files/husk-path-race-sentinel
cleanup() {
    adb -s "$DEV" shell "run-as $PKG rm -f '$SENTINEL'" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

adb -s "$DEV" shell "run-as $PKG sh -c 'printf HOST_SENTINEL_DO_NOT_TOUCH > \"$SENTINEL\"; chmod 640 \"$SENTINEL\"; touch -t 202001010101 \"$SENTINEL\"'"
before_meta=$(adb -s "$DEV" shell "run-as $PKG stat -c '%a:%s:%Y' '$SENTINEL'" | tr -d '\r')
before_hash=$(adb -s "$DEV" shell "run-as $PKG sha256sum '$SENTINEL'" | awk '{print $1}')

out=$(DEV="$DEV" PKG="$PKG" ROOTFS="$ROOTFS" LD="$LD" \
    ENV="HOST_SENTINEL=$SENTINEL,NR_FAKE_ROOT=0" EXTRA=../probe/path-race-probe.c \
    sh "$HERE/run-guest.sh" audit-path-race.sh)
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
