#!/bin/sh
# Runs inside the guest: every unit-test binary dropped in /tmp/unit.
# Prints one "== name" header and the test binary's tail per binary.
set -u
status=0
for f in /tmp/unit/*; do
    [ -x "$f" ] || continue
    echo "== $(basename "$f")"
    if ! "$f" --test-threads=1 2>&1 | tail -3; then
        status=1
    fi
done
exit "$status"
