#!/bin/sh
# Run the aarch64-linux-gnu unit-test binaries inside the guest tree.
#
#   DEV=... [PKG=...] [ROOTFS=...] sh tools/device-tests/run-unit-tests.sh
#
# Fails when a binary cannot run or reports failures: a missing test case must
# not look like a pass. Everything goes in through stdin so the device side needs
# no writable /data/local path and no extra quoting.
set -u
DEV=${DEV:?usage: DEV=<adb serial> [PKG=...] [ROOTFS=...] $0}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
OUT=${OUT:-/tmp/unit}
BUILD=${BUILD:-runtime/target/aarch64-unknown-linux-gnu/debug/build}
HERE=$(cd "$(dirname "$0")" && pwd)

exes=$(find "$BUILD" -maxdepth 4 -type f \( -name "gpu_env-*" -o -name "nr_tools-*" -o -name "runtime_core-*" -o -name "v4l2_shim-*" \) \
        ! -name "*.d" ! -name "*.o" ! -name "*.rlib" ! -name "*.rmeta" 2>/dev/null | sort)
[ -n "$exes" ] || { echo "no test binaries under $BUILD; run make test first" >&2; exit 1; }

adb -s "$DEV" shell "run-as $PKG mkdir -p $ROOTFS$OUT" >/dev/null 2>&1
for exe in $exes; do
    b=$(basename "$exe")
    adb -s "$DEV" shell "run-as $PKG sh -c \"cat > $ROOTFS$OUT/$b\"" < "$exe" || exit 1
    adb -s "$DEV" shell "run-as $PKG chmod 755 $ROOTFS$OUT/$b" || exit 1
done

adb -s "$DEV" shell "run-as $PKG sh -c \"cat > $ROOTFS/tmp/run-unit-tests.sh\"" < "$HERE/unit-runner-guest.sh" || exit 1
adb -s "$DEV" shell "run-as $PKG chmod 755 $ROOTFS/tmp/run-unit-tests.sh" || exit 1

log=$(mktemp "${TMPDIR:-/tmp}/husk-unit-run.XXXXXX")
trap 'rm -f "$log"' EXIT HUP INT TERM

adir=$(dirname "$ROOTFS")
adb -s "$DEV" shell "run-as $PKG sh -c \"cd $adir && ./husk run ubuntu -c /tmp/run-unit-tests.sh\"" >"$log" 2>&1
rc=$?
cat "$log"

FAIL=0
grep -q "test result: ok" "$log" || { echo "UNIT: no test result line at all" >&2; FAIL=1; }
if grep -qE "test result: FAILED|[1-9][0-9]* failed" "$log"; then echo "UNIT: failures reported" >&2; FAIL=1; fi
[ "$rc" -eq 0 ] || { echo "UNIT: runner exited $rc" >&2; FAIL=1; }

if [ "$FAIL" -eq 0 ]; then echo "UNIT_TESTS: PASS"; else echo "UNIT_TESTS: FAIL"; fi
exit "$FAIL"
