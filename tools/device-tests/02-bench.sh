#!/bin/bash
# Micro-benchmarks of the path layer. Driven by run-guest.sh.
# The measurement itself is a guest binary (runtime/crates/guest-bench, Rust),
# built by tools/guest-config/build-guest-tools.sh. The shell is never in the
# timed path, and there is no C fallback: the guest has no usable libc headers.
#
# Reading the numbers:
#   stat-real / open-real   passthrough: our interception, but no resolution.
#   stat-guest / open-guest resolution: same syscall plus walking every component
#                           against the root fd (symlink-aware) and re-composing
#                           the real path.
#   The difference is the resolver. There is no third cost: no trap, no IPC, no
#   ptrace, and the kernel sees exactly the calls a native process makes.
set -e
cd /tmp
# The Rust guest binary is pushed as EXTRA=bench; build it with
#   sh tools/guest-config/build-guest-tools.sh
if [ ! -x /bench ]; then
  echo "missing /bench: push runtime/crates/guest-bench (EXTRA=bench)" >&2
  exit 1
fi
cp /bench ./bench
for m in syscall stat-real stat-guest stat-deep open-real open-guest exec; do ./bench "$m"; done
echo "== correctness smoke"
head -1 /etc/os-release
cd /etc && pwd -P
