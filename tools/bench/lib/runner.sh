#!/bin/sh
# Runner adapters. Everything else in the harness is runner-agnostic.
#
#   husk          in-process loader + path shim (no ptrace, no namespaces)
#   proot         the proot binary LinuxHub 2.0 shipped, with 2.0's exact flags
#   proot-distro  the same binary with proot-distro's invocation profile
#
# All three are started from the app's own uid, same cwd (/root in the guest),
# same rootfs tree.
set -eu
# Loaded with `set -u` off on purpose: sourced, so $0 is the caller (run-all.sh).
. "${BENCH_ROOT:?set BENCH_ROOT to tools/bench}/bench.env"

# adb baseline: the cost of `adb shell` + `run-as`, subtracted from cold-start
# numbers so the table measures the runner rather than the USB link.
run_host() {
  adb -s "$DEV" shell "run-as $PKG sh -c '$*'"
}

run_husk() {
  adb -s "$DEV" shell "run-as $PKG sh $BENCH_DIR/husk-run.sh '$*'"
}

run_proot() {
  adb -s "$DEV" shell "run-as $PKG sh $BENCH_DIR/proot-run.sh '$*'"
}

run_proot_distro() {
  adb -s "$DEV" shell "run-as $PKG sh $BENCH_DIR/prootdistro-run.sh '$*'"
}

run_with() {
  case "$1" in
    husk)         run_husk "$2" ;;
    proot)        run_proot "$2" ;;
    proot-distro) run_proot_distro "$2" ;;
    *) echo "unknown runner: $1" >&2; return 2 ;;
  esac
}
