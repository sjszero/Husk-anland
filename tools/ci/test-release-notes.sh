#!/bin/sh
# Verify the canonical GitHub Release body generated from CHANGELOG.md.
set -eu
cd "$(dirname "$0")/../.."

version=$(cat VERSION)
tmp=$(mktemp "${TMPDIR:-/tmp}/husk-release-notes.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM

sh tools/ci/release-notes.sh "$version" > "$tmp"

first=$(sed -n '/[^[:space:]]/{p;q;}' "$tmp")
if [ "$first" != "## 概要" ]; then
    echo "release-notes test: first non-empty line is '$first', expected '## 概要'" >&2
    exit 1
fi

if grep -Eq '^Husk (在 Android 上|是)|^Linux 运行环境|^> 主题：' "$tmp"; then
    echo "release-notes test: project introduction or theme banner leaked into release body" >&2
    exit 1
fi

if grep -Eq '^### ' "$tmp"; then
    echo "release-notes test: subsection headings must be level two" >&2
    exit 1
fi

if grep -Eq '^#{1,6} .*v?[0-9]+\.[0-9]+\.[0-9]+' "$tmp"; then
    echo "release-notes test: release body repeats a version heading" >&2
    exit 1
fi

# ERE, not BRE alternation: `a\|b` is a GNU extension and BSD grep answers
# "no match" for it, so this check passed on CI and failed on every macOS
# checkout with a body that was perfectly fine (measured 2026-11-26).
if ! grep -Eq '^## (修复|变更|验证)$' "$tmp"; then
    echo "release-notes test: release body has no expected section heading" >&2
    exit 1
fi

if sh tools/ci/release-notes.sh 0.0.100 >/dev/null 2>&1; then
    echo "release-notes test: accepted version component above 99" >&2
    exit 1
fi

echo "release-notes: PASS ($version)"
