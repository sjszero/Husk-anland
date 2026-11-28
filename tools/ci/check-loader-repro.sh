#!/bin/sh
# Two builds of the same inputs must produce the same bytes.
#
# Why: evidence 350 §3 recorded one build of 869888 B whose publish_exe_guest()
# worked (/proc/self/exe -> /usr/bin/readlink) and another of 738816 B where it
# did not (/proc/self/exe fell back to the host loader path). Only 3 of the 38
# acceptance checks noticed, so "gate A is green" cannot be the only guard.
# A byte-diff between two builds of the same tree catches that class before it
# reaches a device (defect H37).
#
# Needs NDK. Builds into temporary OUT directories, so jniLibs is untouched.
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
: "${NDK:?set NDK=<ndk>/bin}"
A=$(mktemp -d)
B=$(mktemp -d)
trap 'rm -rf "$A" "$B"' EXIT
OUT="$A" sh "$ROOT/tools/loader/build-embedded-loader.sh" >"$A/build.log" 2>&1 || {
    echo "loader repro: FAIL (first build failed; see $A/build.log)" >&2
    tail -20 "$A/build.log" >&2
    exit 1
}
OUT="$B" sh "$ROOT/tools/loader/build-embedded-loader.sh" >"$B/build.log" 2>&1 || {
    echo "loader repro: FAIL (second build failed; see $B/build.log)" >&2
    tail -20 "$B/build.log" >&2
    exit 1
}
[ -f "$A/libld.so" ] && [ -f "$B/libld.so" ] || {
    echo "loader repro: FAIL (build did not produce libld.so in OUT)" >&2
    exit 1
}
sa=$(sha256sum "$A/libld.so" | cut -d' ' -f1)
sb=$(sha256sum "$B/libld.so" | cut -d' ' -f1)
size=$(wc -c <"$A/libld.so" | tr -d ' ')
if [ "$sa" != "$sb" ]; then
    echo "loader repro: FAIL (non-deterministic build: $sa vs $sb, $size B)" >&2
    exit 1
fi
echo "loader repro: PASS ($sa, $size B)"
