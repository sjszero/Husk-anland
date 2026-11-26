#!/bin/sh
# Guest-side launcher for the concurrent final-symlink replacement probe.
set -eu
: "${HOST_SENTINEL:?HOST_SENTINEL must be the host-side sentinel path}"
gcc -O2 -pthread -Wall -Wextra -o /tmp/path-race-probe /tmp/path-race-probe.c
/tmp/path-race-probe "$HOST_SENTINEL"
rm -f /tmp/path-race-probe /tmp/path-race-probe.c
