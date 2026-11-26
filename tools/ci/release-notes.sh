#!/bin/sh
# Print one CHANGELOG section without its heading.
#
# The GitHub Release page already renders the release title. Repeating the same
# version heading as the first line of the body makes the title page inconsistent
# with the repository and with older releases.
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
