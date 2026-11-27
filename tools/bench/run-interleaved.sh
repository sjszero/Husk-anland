#!/bin/sh
# run-interleaved.sh [reps] [runners]
#
# Round-robin instead of "one runner to completion, then the next". Running the
# three runners in separate blocks takes ~25 minutes, and this phone throttles:
# the husk block re-run at 16:32 was 1.5-1.8x slower than the same block at 16:05
# on stat/find/tar/make. Interleaving makes thermal and page-cache drift hit all
# three runners equally instead of being charged to whichever ran last.
#
#   runners: default "husk proot proot-distro"
#   reps:    default 5
#
# Writes docs/design/evidence/bench-<runner>-<date>-il.log, one per runner, and
# records thermal zone max + battery level before and after every block so the
# table can state the temperature and charge range it was taken in.
set -eu
. "$(dirname "$0")/bench.env"
HERE=$(cd "$(dirname "$0")" && pwd)
BENCH_ROOT=$HERE
export BENCH_ROOT
. "$HERE/lib/runner.sh"
REPO_ROOT=$(cd "$HERE/../.." && pwd)
HUSK_TOOLS=${HUSK_TOOLS:-$REPO_ROOT/runtime/target/debug/husk-tools}
[ -x "$HUSK_TOOLS" ] || {
  echo "run-interleaved: missing $HUSK_TOOLS (run make tools)" >&2
  exit 1
}

REPS=${1:-$REPS}
RUNNERS=${2:-husk proot proot-distro}
DATE=$(date +%Y%m%d-%H%M)
OUTDIR=$(cd "$HERE/../.." && pwd)/docs/design/evidence

ms() { "$HUSK_TOOLS" now-ms; }

# Thermal max + battery, so "same conditions" is a measurement and not a claim.
state() {
  t=$(adb -s "$DEV" shell 'cat /sys/class/thermal/thermal_zone*/temp 2>/dev/null' 2>/dev/null \
      | tr -d '\r' | sort -n | tail -1)
  b=$(adb -s "$DEV" shell 'dumpsys battery' 2>/dev/null \
      | sed -n 's/^  level: //p;s/^  temperature: //p' | tr '\n' ' ')
  echo "# state $1 max_thermal=$t battery=$b"
}

for r in $RUNNERS; do
  : > "$OUTDIR/bench-$r-$DATE-il.log"
  {
    echo "# bench runner=$r reps=$REPS interleaved date=$DATE"
    echo "# device=$DEV pkg=$PKG rootfs=$ROOTFS"
  } >> "$OUTDIR/bench-$r-$DATE-il.log"
done

# Fixtures once per runner (idempotent; every runner sees the same trees).
for r in $RUNNERS; do
  run_with "$r" 'sh /tmp/prep.sh' >/dev/null 2>&1 || true
done

rep=1
while [ "$rep" -le "$REPS" ]; do
  if [ "$rep" -le 2 ]; then cold=1; else cold=0; fi
  for r in $RUNNERS; do
    f="$OUTDIR/bench-$r-$DATE-il.log"
    { echo "#rep $rep"; state "before rep$rep"; } >> "$f"
    # cold start every rep: it is one exec, cheap, and it is the dimension most
    # sensitive to who ran first, so it needs the same n as everything else.
    t0=$(ms)
    run_with "$r" '/bin/true' >/dev/null 2>&1 || echo "  cold_start failure" >> "$f"
    t1=$(ms)
    echo "cold_start_ms\t$((t1 - t0))" >> "$f"
    run_with "$r" "env BENCH_COLD_APT=$cold sh /tmp/measure.sh" >> "$f" 2>&1
    state "after rep$rep" >> "$f"
    sleep 60
  done
  rep=$((rep + 1))
done

for r in $RUNNERS; do
  echo "=== $r"
  "$HUSK_TOOLS" bench-summarize "$OUTDIR/bench-$r-$DATE-il.log"
done
