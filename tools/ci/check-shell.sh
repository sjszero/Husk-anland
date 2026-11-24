#!/bin/sh
# `sh -n` over every shipped script: catches the class of syntax error that only
# shows up on a device, where the shell is mksh and the error text is unhelpful.
set -eu
cd "$(dirname "$0")/../.."
n=0
fail=0
for f in $(find tools -name '*.sh' -type f); do
    n=$((n + 1))
    sh -n "$f" || { echo "syntax error: $f" >&2; fail=1; }
done
echo "shell syntax: $n script(s) checked"
[ "$fail" -eq 0 ] || exit 1
