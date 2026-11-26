#!/bin/sh
# Exercise the release number contract without touching the real VERSION file.
#
# Husk uses base-100 components.  The carry boundary is the part that must not
# drift: 0.0.99 is followed by 0.1.0, and 0.99.99 by 1.0.0.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d "${TMPDIR:-/tmp}/husk-version-bump.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp/tools/ci"
cp tools/ci/version-bump.sh "$tmp/tools/ci/version-bump.sh"

check_next() {
    old=$1
    expected=$2
    printf '%s\n' "$old" > "$tmp/VERSION"
    if ! (cd "$tmp" && sh tools/ci/version-bump.sh >/dev/null); then
        echo "version-bump test: rejected valid version $old" >&2
        exit 1
    fi
    actual=$(cat "$tmp/VERSION")
    if [ "$actual" != "$expected" ]; then
        echo "version-bump test: $old -> $actual, expected $expected" >&2
        exit 1
    fi
    printf 'version-bump test: %s -> %s\n' "$old" "$actual"
}

check_rejected() {
    old=$1
    printf '%s\n' "$old" > "$tmp/VERSION"
    if (cd "$tmp" && sh tools/ci/version-bump.sh >/dev/null 2>&1); then
        echo "version-bump test: accepted invalid version $old" >&2
        exit 1
    fi
    printf 'version-bump test: reject %s\n' "$old"
}

check_next 0.0.32 0.0.33
check_next 0.0.98 0.0.99
check_next 0.0.99 0.1.0
check_next 0.99.99 1.0.0
check_rejected 0.0.100
check_rejected 0.0.1.2

echo "version-bump: PASS"
