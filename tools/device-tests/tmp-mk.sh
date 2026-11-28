#!/bin/sh
cd /root/bench-make || exit 1
ls | head -3; echo "--- Makefile ---"; head -12 Makefile
rm -f *.o bench
s=$(date +%s%3N); make -j4 bench 2>&1 | tail -3; e=$(date +%s%3N)
echo "make_j4_ms=$((e-s))"; ls -l bench 2>/dev/null | head -1
