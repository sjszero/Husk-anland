#!/bin/sh
# Keep every active runtime surface on the shipped Android package id.
#
# Historical evidence and released CHANGELOG entries keep their original package
# names because they record what actually ran at that time. Live scripts and
# current documentation must never drift back to a retired package: a stale
# default can make a test read or write another installed app.
set -eu
cd "$(dirname "$0")/../.."

canonical=com.linuxhub.husk
tmp=$(mktemp -d "${TMPDIR:-/tmp}/husk-package-audit.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
fail=0

# Collect every com.linuxhub.*husk occurrence outside immutable history.
git grep --untracked -n -E 'com\.linuxhub\.[A-Za-z0-9_.-]*husk' \
    -- ':!docs/design/evidence/**' ':!CHANGELOG.md' > "$tmp/ids" || true

# A line may contain the canonical id and an unrelated stale id. Remove the
# canonical spelling first, then reject any second com.linuxhub.*husk on it.
awk -v canonical="$canonical" '
    {
        line = $0
        gsub(canonical, "", line)
        if (line ~ /com\.linuxhub\.[A-Za-z0-9_.-]*husk/) print
    }
' "$tmp/ids" > "$tmp/noncanonical"

if [ -s "$tmp/noncanonical" ]; then
    echo "runtime package audit: noncanonical package id" >&2
    sed 's/^/  /' "$tmp/noncanonical" >&2
    fail=1
fi

check_literal() {
    file=$1
    expected=$2
    label=$3
    if ! grep -Fqx "$expected" "$file"; then
        echo "runtime package audit: $label is not pinned to $canonical" >&2
        echo "  expected in $file: $expected" >&2
        fail=1
    fi
}

check_literal Makefile 'PKG        ?= com.linuxhub.husk' 'Makefile PKG default'
check_literal app/app/build.gradle.kts '    namespace = "com.linuxhub.husk"' 'Android namespace'
check_literal app/app/build.gradle.kts '        applicationId = "com.linuxhub.husk"' 'Android applicationId'
check_literal runtime/crates/nr-tools/src/provision.rs '    let mut app = "com.linuxhub.husk".to_string();' 'nr-tools provision default'
check_literal tools/bench/bench.env 'PKG=${PKG:-com.linuxhub.husk}' 'benchmark package default'

if [ "$fail" -ne 0 ]; then
    echo "runtime package audit: FAIL" >&2
    exit 1
fi

echo "runtime package audit: PASS ($canonical)"
