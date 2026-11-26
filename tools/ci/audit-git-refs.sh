#!/bin/sh
# Reject documentation that points at an unpublished or deleted release tag.
#
# Historical source snapshots must use a commit id or a tag that still exists.
# This catches stale `git show vX.Y.Z:path` examples after tag cleanup. Evidence
# files are excluded because they intentionally quote failures from older trees.
set -eu
cd "$(dirname "$0")/../.."

tmp=$(mktemp "${TMPDIR:-/tmp}/husk-git-refs.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM

git grep --untracked -n -E 'git show v[0-9]+\.[0-9]+\.[0-9]+:' -- . \
    ':(exclude)CHANGELOG.md' ':(exclude)docs/design/evidence/**' > "$tmp" || true

bad=0
while IFS=: read -r file line text; do
    [ -n "${file:-}" ] || continue
    tag=$(printf '%s\n' "$text" | sed -n 's/.*git show \(v[0-9][0-9.]*\):.*/\1/p')
    [ -n "$tag" ] || continue
    if ! git rev-parse --verify --quiet "refs/tags/$tag^{commit}" >/dev/null; then
        echo "git reference: $file:$line points at missing tag $tag" >&2
        bad=1
    fi
done < "$tmp"

if [ "$bad" -ne 0 ]; then
    exit 1
fi
echo "git references: PASS"
