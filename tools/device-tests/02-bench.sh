#!/bin/bash
# Micro-benchmarks of the path layer. Driven by run-guest.sh.
# The measurement itself is bench.c, compiled with the guest gcc and run in the
# guest - the shell is not in the timed path.
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
cp /bench.c .
gcc -O2 -o bench bench.c
for m in syscall stat-real stat-guest stat-deep open-real open-guest exec; do ./bench "$m"; done
echo "== correctness smoke"
head -1 /etc/os-release
cd /etc && pwd -P
