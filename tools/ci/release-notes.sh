#!/bin/sh
# Print one CHANGELOG section in the canonical GitHub Release body format.
#
# The release title is `Husk X.Y.Z`.  The page body starts with `## 概要`, never
# with a repeated version heading or the project-wide introduction.  CHANGELOG
# subsections are promoted to level-two headings so every historical release can
# use the same outline.
set -eu
cd "$(dirname "$0")/../.."

version=${1:-$(cat VERSION)}
case "$version" in
    ''|*[!0-9.]*|.*|*.|*..*) echo "release-notes: invalid version: $version" >&2; exit 65 ;;
esac

old_ifs=$IFS
IFS=.
set -- $version
IFS=$old_ifs
[ "$#" -eq 3 ] || { echo "release-notes: expected X.Y.Z, got $version" >&2; exit 65; }
for part in "$1" "$2" "$3"; do
    [ "$part" -le 99 ] || {
        echo "release-notes: version component exceeds 99: $version" >&2
        exit 65
    }
done

{
    awk -v marker="## [$version]" '
        index($0, marker) == 1 { found = 1; next }
        found && /^## \[/ { exit }
        found && !started && $0 == "" { next }
        found { started = 1; print }
        END { if (!found) exit 1 }
    ' CHANGELOG.md || {
        echo "release-notes: CHANGELOG has no section for $version" >&2
        exit 66
    }
} | awk '
    BEGIN {
        print "## 概要"
        print ""
        summary = 1
    }
    summary && /^### / {
        print ""
        summary = 0
        sub(/^### /, "## ")
        print
        next
    }
    summary {
        if ($0 == "") next
        if ($0 ~ /^> 主题：/) {
            sub(/^> 主题：/, "")
            gsub(/\*\*/, "")
            print
            next
        }
        if ($0 ~ /^> /) {
            sub(/^> /, "")
            gsub(/\*\*/, "")
            if ($0 ~ /^原始输出见 /) next
            print
            next
        }
        print
        next
    }
    {
        sub(/^### /, "## ")
        print
    }
'
