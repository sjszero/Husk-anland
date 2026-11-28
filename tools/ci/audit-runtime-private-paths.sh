#!/bin/sh
# Fail the release if a runtime artifact embeds the build host's private paths.
# Rust panic/debug location strings survive strip, so they must be remapped at
# compile time and checked on the final tarball, not only on source files.
set -eu

ARCHIVE=${1:?usage: audit-runtime-private-paths.sh <runtime.tar.gz>}
[ -f "$ARCHIVE" ] || { echo "private paths: missing $ARCHIVE" >&2; exit 1; }

HOST_HOME=${HOME:-}
if [ -n "$HOST_HOME" ]; then
    PATTERN=$(printf '%s\n' "$HOST_HOME" '/Users/' '/tmp/nr-toolchain' '/var/folders/' | awk 'NF' | paste -sd'|' -)
else
    PATTERN='/Users/|/tmp/nr-toolchain|/var/folders/'
fi

bad=0
for entry in $(tar tzf "$ARCHIVE"); do
    case "$entry" in */) continue ;; esac
    # -a makes grep treat ELF bytes as text; matching is intentionally narrow.
    if tar xzOf "$ARCHIVE" "$entry" 2>/dev/null | grep -aEq "$PATTERN"; then
        echo "private paths: $entry" >&2
        bad=1
    fi
done

[ "$bad" -eq 0 ] || {
    echo "private paths: FAIL ($ARCHIVE)" >&2
    exit 1
}
echo "private paths: PASS ($ARCHIVE)"
