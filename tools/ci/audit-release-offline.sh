#!/bin/sh
# 发布命名契约的「离线半」：不联网、不需要 gh，CI 每次 push 都跑。
#
# 为什么拆开：以前这些规则全写在 tools/ci/audit-release-names.sh 里，而那个脚本
# 要 gh 认证、只在发版当天人工跑。结果是"版本三元组、tag 命名、序列不跳号、
# VERSION 与最新 tag 一致、README 首屏头块"这些**根本不需要网络**的规则，一次都
# 没进过 CI（第 2 轮缺陷 H4）。
#
# 联网半仍然是 tools/ci/audit-release-names.sh（人工）：它先调本脚本，再查只有
# GitHub 知道的东西 —— annotated tag 的注释正文、Release 标题与正文结构、资产名。
# 两半共用这里的规则，不各自实现一遍。
#
# 本地 tag 视图：CI 里要 actions/checkout 带 fetch-depth: 0，否则 tag 列表是空的，
# 这里会明确打 SKIP 而不是当成通过。
set -eu
cd "$(dirname "$0")/../.."
fail=0
version=$(cat VERSION)

# 1. VERSION 本身：100 进制三元组，每段 0..99，无前导零。
if ! printf '%s\n' "$version" | awk -F. '
    NF != 3 { exit 1 }
    $1 !~ /^(0|[1-9][0-9]*)$/ ||
    $2 !~ /^(0|[1-9][0-9]*)$/ ||
    $3 !~ /^(0|[1-9][0-9]*)$/ { exit 1 }
    $1 > 99 || $2 > 99 || $3 > 99 { exit 1 }
'; then
    echo "VERSION is not a 100-carry triple (0..99 per component, no leading zeros): $version" >&2
    fail=1
fi

# 2. 首页首屏：头块里不能有散文，项目介绍不许爬回 "## 概览" 上面。
if ! sh tools/ci/audit-readme-header.sh; then
    fail=1
fi

# 3. tag 命名与序列（本地视图）。
tags=$(git tag -l)
if [ -z "$tags" ]; then
    echo "SKIP tag sequence: no local tags (CI needs actions/checkout with fetch-depth: 0)"
else
    printf '%s\n' "$tags" | sort -u > "${TMPDIR:-/tmp}/husk-offline-tag-names.$$"
    trap 'rm -f "${TMPDIR:-/tmp}/husk-offline-tag-names.$$"' EXIT HUP INT TERM

    while IFS= read -r tag; do
        case "$tag" in
            v[0-9]*.[0-9]*.[0-9]*) ;;
            *)
                echo "invalid tag name: $tag (expected vX.Y.Z)" >&2
                fail=1
                ;;
        esac
    done < "${TMPDIR:-/tmp}/husk-offline-tag-names.$$"

    sed 's/^v//' "${TMPDIR:-/tmp}/husk-offline-tag-names.$$" \
        | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' \
        | sort -t. -k1,1n -k2,2n -k3,3n > "${TMPDIR:-/tmp}/husk-offline-tag-versions.$$"

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
    done < "${TMPDIR:-/tmp}/husk-offline-tag-versions.$$"

    if [ "$first" -eq 1 ]; then
        echo "SKIP tag sequence: no version tags among $(printf '%s' "$tags" | wc -l | tr -d ' ') local tags"
    else
        # VERSION 只有两种合法状态：等于最新 tag（已发版），或者正好是最新 tag 的
        # 下一个号（这一版正在准备、还没打 tag）。后者必须允许，否则"准备发版"
        # 本身就会让门禁变红；前者之外的一切（跳号、回退）都是错。
        next_version=$(printf '%s\n' "$last_version" | awk -F. '{
            p = $3 + 1; m = $2; j = $1;
            if (p == 100) { p = 0; m = m + 1 }
            if (m == 100) { m = 0; j = j + 1 }
            printf "%d.%d.%d\n", j, m, p
        }')
        if [ "$version" != "$last_version" ] && [ "$version" != "$next_version" ]; then
            echo "VERSION is $version, but the newest tag is v$last_version (allowed: $last_version or $next_version)" >&2
            fail=1
        else
            echo "VERSION $version is consistent with newest tag v$last_version"
        fi
    fi
fi

if [ "$fail" -ne 0 ]; then
    echo "release offline audit: FAIL (version=$version)" >&2
    exit 1
fi
echo "release offline audit: PASS (version=$version)"
