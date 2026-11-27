#!/bin/sh
# tools/bench/run-all.sh <runner> [reps]
#
#   runner: husk | proot | proot-distro
#   reps:   default 5 (median + range are reported, per the release goal)
#
# Writes the raw output to docs/design/evidence/bench-<runner>-<date>.log and
# prints a median table. Run it once per runner; docs/BENCHMARKS.md is built
# from those three logs.
set -eu
. "$(dirname "$0")/bench.env"
HERE=$(cd "$(dirname "$0")" && pwd)
BENCH_ROOT=$HERE
export BENCH_ROOT
. "$HERE/lib/runner.sh"
REPO_ROOT=$(cd "$HERE/../.." && pwd)
HUSK_TOOLS=${HUSK_TOOLS:-$REPO_ROOT/runtime/target/debug/husk-tools}
[ -x "$HUSK_TOOLS" ] || {
  echo "run-all: missing $HUSK_TOOLS (run make tools)" >&2
  exit 1
}

RUNNER=${1:?usage: run-all.sh <husk|proot|proot-distro> [reps]}
REPS=${2:-$REPS}
DATE=$(date +%Y%m%d-%H%M)
OUT="$(cd "$HERE/../.." && pwd)/docs/design/evidence/bench-$RUNNER-$DATE.log"

baseline_ms() {
  # `adb shell` + `run-as` + a host /system/bin/true: subtracted from cold start.
  i=0
  while [ $i -lt "$REPS" ]; do
    i=$((i+1))
    t0=$("$HUSK_TOOLS" now-ms)
    adb -s "$DEV" shell "run-as $PKG /system/bin/true" >/dev/null 2>&1
    t1=$("$HUSK_TOOLS" now-ms)
    echo $((t1 - t0))
  done
}

{
  echo "# bench runner=$RUNNER reps=$REPS date=$DATE"
  echo "# device=$DEV pkg=$PKG rootfs=$ROOTFS"
  echo "## baseline_ms (adb+run-as+/system/bin/true)"
  baseline_ms >&2

  echo "## cold_start_ms (runner -> /bin/true, wall clock, includes adb)"
  i=0
  while [ $i -lt "$REPS" ]; do
    i=$((i+1))
    t0=$("$HUSK_TOOLS" now-ms)
    run_with "$RUNNER" '/bin/true' >/dev/null 2>&1 || echo "  cold_start failure"
    t1=$("$HUSK_TOOLS" now-ms)
    echo $((t1 - t0))
  done

  echo "## measure (runs inside the guest, one block per repetition)"
  # One-time fixtures (200-file C project, 1000-file git repo) so every runner
  # sees the same trees.
  echo "# prep" >&2
  run_with "$RUNNER" "sh /tmp/prep.sh" >&2 2>&1 || true
  i=0
  while [ $i -lt "$REPS" ]; do
    i=$((i+1))
    echo "#rep $i"
    # Cold apt: drop the index and re-download, on the first two reps only.
    if [ "$i" -le 2 ]; then cold=1; else cold=0; fi
    run_with "$RUNNER" "env BENCH_COLD_APT=$cold sh /tmp/measure.sh" 2>&1
    sleep 60   # cooldown between repetitions, as the goal requires
  done
} > "$OUT" 2>&1

echo "raw: $OUT"
"$HUSK_TOOLS" bench-summarize "$OUT"
