#!/bin/sh
# Negative/positive test for audit-runtime-private-paths.sh. The release gate
# must reject a runtime artifact containing a developer-machine path, and must
# accept an equivalent clean artifact.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d "${TMPDIR:-/tmp}/husk-runtime-private-paths-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

valid="$tmp/valid"
mkdir -p "$valid/guest"
printf 'runtime payload\n' > "$valid/guest/libexample.so"
tar czf "$tmp/valid.tar.gz" -C "$valid" .
if ! sh tools/ci/audit-runtime-private-paths.sh "$tmp/valid.tar.gz" > "$tmp/valid.out" 2>&1; then
    cat "$tmp/valid.out" >&2
    echo "runtime private paths test: clean bundle was rejected" >&2
    exit 1
fi

bad="$tmp/bad"
mkdir -p "$bad/guest"
printf 'source=/Users/alice/private-path/example.rs\n' > "$bad/guest/libexample.so"
tar czf "$tmp/bad.tar.gz" -C "$bad" .
if sh tools/ci/audit-runtime-private-paths.sh "$tmp/bad.tar.gz" > "$tmp/bad.out" 2>&1; then
    cat "$tmp/bad.out" >&2
    echo "runtime private paths test: private path was accepted" >&2
    exit 1
fi
grep -q 'private paths: .*guest/libexample.so' "$tmp/bad.out" || {
    cat "$tmp/bad.out" >&2
    echo "runtime private paths test: wrong rejection reason" >&2
    exit 1
}

echo "runtime private paths test: PASS (clean accepted, private path rejected)"
