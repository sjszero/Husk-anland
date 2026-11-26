#!/bin/sh
# Audit published tag and GitHub Release naming.
#
# This is a network-backed release gate, not part of ordinary CI: it requires an
# authenticated `gh` CLI. It catches duplicate/non-version tags and title drift
# that a local VERSION/CHANGELOG check cannot see.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d "${TMPDIR:-/tmp}/husk-release-audit.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
fail=0
version=$(cat VERSION)

git ls-remote --tags origin > "$tmp/remote-tags"
awk '$2 ~ /^refs\/tags\// {
    tag = $2
    sub(/^refs\/tags\//, "", tag)
    sub(/\^\{\}$/, "", tag)
    print tag
}' "$tmp/remote-tags" | sort -u > "$tmp/tag-names"

tag_count=0
while IFS= read -r tag; do
    case "$tag" in
        v[0-9]*.[0-9]*.[0-9]*) ;;
        *)
            echo "invalid tag name: $tag (expected vX.Y.Z)" >&2
            fail=1
            continue
            ;;
    esac

    number=${tag#v}
    if ! printf '%s\n' "$number" | awk -F. '
        NF != 3 { exit 1 }
        $1 !~ /^(0|[1-9][0-9]*)$/ ||
        $2 !~ /^(0|[1-9][0-9]*)$/ ||
        $3 !~ /^(0|[1-9][0-9]*)$/ { exit 1 }
        $1 > 99 || $2 > 99 || $3 > 99 { exit 1 }
    '; then
        echo "invalid version tag: $tag (expected vX.Y.Z, components 0..99, no leading zeros)" >&2
        fail=1
        continue
    fi
    tag_count=$((tag_count + 1))
done < "$tmp/tag-names"

repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
gh api "repos/$repo/releases?per_page=100" \
    --jq '.[] | [.tag_name, .name, (.body | split("\n")[0])] | @tsv' \
    > "$tmp/releases.tsv"
cut -f1 "$tmp/releases.tsv" | sort -u > "$tmp/release-tags"

comm -3 "$tmp/tag-names" "$tmp/release-tags" > "$tmp/tag-release-diff"
if [ -s "$tmp/tag-release-diff" ]; then
    echo "tag/release mismatch:" >&2
    sed 's/^/  /' "$tmp/tag-release-diff" >&2
    fail=1
fi

release_count=0
while IFS="$(printf '\t')" read -r tag name first_line; do
    [ -n "$tag" ] || continue
    release_count=$((release_count + 1))
    case "$tag" in
        v[0-9]*.[0-9]*.[0-9]*) expected="Husk ${tag#v}" ;;
        *)
            echo "invalid release tag: $tag" >&2
            fail=1
            continue
            ;;
    esac

    if [ "$name" != "$expected" ]; then
        echo "release title mismatch: $tag has '$name', expected '$expected'" >&2
        fail=1
    fi

    version_heading='^#{1,6}[[:space:]]+(Husk[[:space:]]+)?v?[0-9]+\.[0-9]+\.[0-9]+([^0-9.]|$)'
    if printf '%s\n' "$first_line" | grep -Eq "$version_heading"; then
        echo "release body repeats a version heading: $tag: $first_line" >&2
        fail=1
    fi

    expected_first="Husk ${tag#v}"
    if [ "$first_line" = "$expected_first" ]; then
        echo "release body repeats the release title: $tag" >&2
        fail=1
    fi
done < "$tmp/releases.tsv"

latest="v$version"
if ! grep -Fqx "$latest" "$tmp/release-tags"; then
    echo "latest VERSION has no GitHub Release: $latest" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "release naming audit: FAIL (tags=$tag_count releases=$release_count version=$version)" >&2
    exit 1
fi

echo "release naming audit: PASS (tags=$tag_count releases=$release_count latest=Husk $version)"
