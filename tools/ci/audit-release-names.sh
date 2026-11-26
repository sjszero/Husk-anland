#!/bin/sh
# Audit published tag and GitHub Release naming.
#
# This is a network-backed release gate, not part of ordinary CI: it requires an
# authenticated `gh` CLI. It catches duplicate/non-version tags and title drift
# that a local VERSION/CHANGELOG check cannot see. It also enforces the canonical
# version-page body: `## 概要`, version-specific summary, level-two sections.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp -d "${TMPDIR:-/tmp}/husk-release-audit.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
fail=0
version=$(cat VERSION)

# 离线半：版本三元组、tag 命名、序列不跳号、VERSION 与最新 tag 的一致关系、
# 首页首屏头块。这些不需要网络，已经进 CI（tools/ci/audit-release-offline.sh）；
# 这里再调一次，保证"人工跑的联网检查"包含同样的规则，而不是各写一遍。
if ! sh tools/ci/audit-release-offline.sh; then
    fail=1
fi

# 下面是联网半：只有 GitHub 知道的那些事情。tag 列表仍然取远端视图，因为
# annotated tag 的注释正文要从 GitHub 读。
git ls-remote --tags origin > "$tmp/remote-tags"
awk '$2 ~ /^refs\/tags\// {
    tag = $2
    sub(/^refs\/tags\//, "", tag)
    sub(/\^\{\}$/, "", tag)
    print tag
}' "$tmp/remote-tags" | sort -u > "$tmp/tag-names"
tag_count=$(wc -l < "$tmp/tag-names" | tr -d ' ')

repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)

# A version has one visible name on every surface. Every release tag must be an
# annotated tag whose first annotation line is exactly `Husk X.Y.Z`; a
# lightweight tag would fall back to the commit subject and reintroduce a second
# naming scheme on the tag page.
while IFS= read -r tag; do
    ref="refs/tags/$tag"
    tag_object=$(awk -v ref="$ref" '$2 == ref { print $1 }' "$tmp/remote-tags")
    if [ -z "$tag_object" ]; then
        echo "tag object not found: $tag" >&2
        fail=1
        continue
    fi
    if ! awk -v ref="$ref^{}" '$2 == ref { found = 1 } END { exit !found }' "$tmp/remote-tags"; then
        echo "tag must be annotated: $tag" >&2
        fail=1
        continue
    fi

    if ! tag_subject=$(gh api "repos/$repo/git/tags/$tag_object" \
        --jq '.message | split("\n")[0]' 2>"$tmp/tag-api.err"); then
        echo "failed to read tag annotation: $tag" >&2
        sed 's/^/  /' "$tmp/tag-api.err" >&2
        fail=1
        continue
    fi
    expected_tag_subject="Husk ${tag#v}"
    if [ "$tag_subject" != "$expected_tag_subject" ]; then
        echo "tag annotation subject mismatch: $tag has '$tag_subject', expected '$expected_tag_subject'" >&2
        fail=1
    fi
done < "$tmp/tag-names"

gh api "repos/$repo/releases?per_page=100" \
    --jq '.[] | [
        .tag_name,
        .name,
        ((.body // "") | split("\n") | map(select(test("\\S"))) | .[0] // ""),
        ((.body // "") | split("\n") | map(select(test("\\S"))) | .[1] // ""),
        ([(.body // "") | split("\n")[] | select(test("^### "))] | length),
        ((.body // "") | test("(?m)^#{1,6}[[:space:]]+(Husk[[:space:]]+)?v?[0-9]+\\.[0-9]+\\.[0-9]+([^0-9.]|$)"))
        ,
        ([.assets[].name] | join(","))
    ] | @tsv' \
    > "$tmp/releases.tsv"
cut -f1 "$tmp/releases.tsv" | sort -u > "$tmp/release-tags"

comm -3 "$tmp/tag-names" "$tmp/release-tags" > "$tmp/tag-release-diff"
if [ -s "$tmp/tag-release-diff" ]; then
    echo "tag/release mismatch:" >&2
    sed 's/^/  /' "$tmp/tag-release-diff" >&2
    fail=1
fi

release_count=0
while IFS="$(printf '\t')" read -r tag name first_line second_line h3_count duplicate_version asset_names; do
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

    if [ "$first_line" != "## 概要" ]; then
        echo "release body must start with '## 概要': $tag has '$first_line'" >&2
        fail=1
    fi

    if [ -z "$second_line" ]; then
        echo "release body has no version-specific summary: $tag" >&2
        fail=1
    fi

    case "$second_line" in
        'Husk 在 Android 上'*|'Husk 是'*|'Linux 运行环境'*|'> 主题：'*)
            echo "release body starts with a project introduction or theme banner: $tag" >&2
            fail=1
            ;;
    esac

    if [ "$h3_count" -ne 0 ]; then
        echo "release body uses level-three headings: $tag ($h3_count)" >&2
        fail=1
    fi
    if [ "$duplicate_version" != "false" ]; then
        echo "release body repeats a version heading: $tag" >&2
        fail=1
    fi

    old_ifs=$IFS
    IFS=,
    set -- $asset_names
    IFS=$old_ifs
    has_bundle=0
    has_sums=0
    for asset in "$@"; do
        [ -n "$asset" ] || continue
        case "$asset" in
            husk-arm64.tar.gz) has_bundle=1 ;;
            SHA256SUMS) has_sums=1 ;;
            husk-rootfs-*.tar.gz|husk-*.apk) ;;
            *)
                echo "noncanonical release asset: $tag/$asset" >&2
                fail=1
                ;;
        esac
    done
    if [ "$has_bundle" -ne 1 ] || [ "$has_sums" -ne 1 ]; then
        echo "release assets missing husk-arm64.tar.gz or SHA256SUMS: $tag" >&2
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
