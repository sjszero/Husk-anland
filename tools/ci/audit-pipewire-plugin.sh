#!/bin/sh
# Verify the vendored gstpipewiresrc crash fix before it enters a release.
set -eu

PLUGIN=${1:-vendor/pipewire/libgstpipewire.so}
EXPECTED=5baacf01e9a98f74a0d3fef88f8da16e9d6e54245b82e871a2b6844665ba7f01
EXPECTED_SIZE=200920

[ -f "$PLUGIN" ] || { echo "pipewire plugin: missing $PLUGIN" >&2; exit 1; }

size=$(wc -c < "$PLUGIN" | tr -d ' ')
[ "$size" = "$EXPECTED_SIZE" ] || {
    echo "pipewire plugin: size $size, expected $EXPECTED_SIZE" >&2
    exit 1
}

if command -v sha256sum >/dev/null 2>&1; then
    have=$(sha256sum "$PLUGIN" | cut -d' ' -f1)
else
    have=$(shasum -a 256 "$PLUGIN" | cut -d' ' -f1)
fi
[ "$have" = "$EXPECTED" ] || {
    echo "pipewire plugin: sha256 $have, expected $EXPECTED" >&2
    exit 1
}

# The string is emitted only by the backported redundant-negotiation branch.
grep -aFq 'stream is running and new caps are still acceptable' "$PLUGIN" || {
    echo "pipewire plugin: redundant-negotiation fix marker missing" >&2
    exit 1
}

grep -aFq 'skipping renegotiation' "$PLUGIN" || {
    echo "pipewire plugin: renegotiation skip marker missing" >&2
    exit 1
}

echo "pipewire plugin: PASS ($have, $size B)"
