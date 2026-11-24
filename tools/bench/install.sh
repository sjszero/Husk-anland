#!/bin/sh
# Push the device-side half of the bench harness into the app's private dir.
# Idempotent: every file is rewritten from this checkout.
set -eu
. "$(dirname "$0")/bench.env"
HERE=$(cd "$(dirname "$0")" && pwd)
R="adb -s $DEV shell"

$R "run-as $PKG mkdir -p $BENCH_DIR/proot" >/dev/null
for f in husk-run.sh proot-run.sh prootdistro-run.sh; do
  adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $BENCH_DIR/$f'" < "$HERE/$f"
done
for f in measure.sh prep.sh; do
  adb -s "$DEV" shell "run-as $PKG sh -c 'cat > $ROOTFS/tmp/$f'" < "$HERE/guest/$f"
done
# The window waiter is a Rust guest binary (aarch64), not a script: build it with
#   sh tools/guest-config/build-guest-tools.sh
ROOT=$(cd "$HERE/../.." && pwd)
WAITER="$ROOT/runtime/target/aarch64-unknown-linux-gnu/release/husk-window-wait"
[ -f "$WAITER" ] || { echo "missing $WAITER — run: sh tools/guest-config/build-guest-tools.sh" >&2; exit 1; }
adb -s "$DEV" push "$WAITER" /data/local/tmp/husk-window-wait >/dev/null
$R "run-as $PKG sh -c 'cp /data/local/tmp/husk-window-wait $ROOTFS/tmp/ && chmod 755 $ROOTFS/tmp/husk-window-wait'"
$R "run-as $PKG sh -c 'chmod 755 $BENCH_DIR/*.sh; chmod 755 $ROOTFS/tmp/measure.sh $ROOTFS/tmp/prep.sh'"
echo "installed: $BENCH_DIR/{husk,proot,prootdistro}-run.sh, $ROOTFS/tmp/{measure,prep}.sh, husk-window-wait"
