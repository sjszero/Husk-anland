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

# The landing page is part of the release surface.  Keep the hero block free of
# prose so a project introduction cannot silently move back above "## 概览".
if ! sh tools/ci/audit-readme-header.sh; then
    fail=1
fi

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

# Versions advance one release at a time using base-100 components.  This is
# deliberately stricter than a generic SemVer check: it catches skipped
# releases and the 0.0.100-style mistake before a release page is trusted.
sed 's/^v//' "$tmp/tag-names" \
    | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' \
    | sort -t. -k1,1n -k2,2n -k3,3n > "$tmp/tag-versions"

first=1
last_version=
expected_major=0
expected_minor=0
expected_patch=1
while IFS=. read -r major minor patch; do
    [ -n "$major" ] || continue
    if [ "$first" -eq 1 ]; then
        if [ "$major" -ne 0 ] || [ "$minor" -ne 0 ] || [ "$patch" -ne 1 ]; then
            echo "release sequence must start at 0.0.1, found $major.$minor.$patch" >&2
            fail=1
        fi
        first=0
    elif [ "$major" -ne "$expected_major" ] || \
         [ "$minor" -ne "$expected_minor" ] || \
         [ "$patch" -ne "$expected_patch" ]; then
        echo "release sequence gap: expected $expected_major.$expected_minor.$expected_patch, found $major.$minor.$patch" >&2
        fail=1
    fi

    expected_patch=$((patch + 1))
    if [ "$expected_patch" -eq 100 ]; then
        expected_patch=0
        expected_minor=$((minor + 1))
        if [ "$expected_minor" -eq 100 ]; then
            expected_minor=0
            expected_major=$((major + 1))
        else
            expected_major=$major
        fi
    else
        expected_minor=$minor
        expected_major=$major
    fi
    last_version="$major.$minor.$patch"
done < "$tmp/tag-versions"

if [ "$first" -eq 1 ]; then
    echo "release sequence is empty" >&2
    fail=1
elif [ "$last_version" != "$version" ]; then
    echo "latest tag is v$last_version but VERSION is $version" >&2
    fail=1
fi

repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)

# Tag annotations are not Release titles. Historical tags predate the strict
# contract; every new tag from 0.0.33 onward must use the exact visible subject.
tag_contract_start=0.0.33
if awk -v have="$version" -v start="$tag_contract_start" '
    function cmp(a, b, i, aa, bb) {
        split(a, aa, "."); split(b, bb, ".")
        for (i = 1; i <= 3; i++) {
            if (aa[i] + 0 < bb[i] + 0) return -1
            if (aa[i] + 0 > bb[i] + 0) return 1
        }
        return 0
    }
    BEGIN { exit cmp(have, start) < 0 }
'; then
    latest_ref="refs/tags/v$version"
    tag_object=$(awk -v ref="$latest_ref" '$2 == ref { print $1 }' "$tmp/remote-tags")
    if [ -z "$tag_object" ]; then
        echo "latest tag object not found: v$version" >&2
        fail=1
    elif awk -v ref="$latest_ref^{}" '$2 == ref { found = 1 } END { exit !found }' "$tmp/remote-tags"; then
        tag_subject=$(gh api "repos/$repo/git/tags/$tag_object" --jq '.message | split("\n")[0]')
        if [ "$tag_subject" != "Husk $version" ]; then
            echo "tag annotation subject mismatch: v$version has '$tag_subject', expected 'Husk $version'" >&2
            fail=1
        fi
    else
        echo "latest tag must be annotated: v$version" >&2
        fail=1
    fi
fi

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
