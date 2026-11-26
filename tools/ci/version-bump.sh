#!/bin/sh
# Advance the Husk release sequence by one.
#
# Husk uses SemVer-shaped versions with a 100-carry release sequence:
#   0.0.98 -> 0.0.99 -> 0.1.0
#   0.99.99 -> 1.0.0
#
# This script changes VERSION only. Add the matching CHANGELOG heading and
# release link, then run `make version-check`.
set -eu
cd "$(dirname "$0")/../.."

old=$(cat VERSION)
case "$old" in
    *[!0-9.]*|.*|*.|*..*) echo "version-bump: invalid VERSION: $old" >&2; exit 65 ;;
esac
old_ifs=$IFS
IFS=.
set -- $old
IFS=$old_ifs
[ "$#" -eq 3 ] || { echo "version-bump: expected X.Y.Z, got $old" >&2; exit 65; }
major=$1
minor=$2
patch=$3
for part in "$major" "$minor" "$patch"; do
    [ "$part" -le 99 ] || {
        echo "version-bump: component exceeds 99 (100 carries): $old" >&2
        exit 65
    }
done

patch=$((patch + 1))
if [ "$patch" -eq 100 ]; then
    patch=0
    minor=$((minor + 1))
    if [ "$minor" -eq 100 ]; then
        minor=0
        major=$((major + 1))
    fi
fi

new="$major.$minor.$patch"
printf '%s\n' "$new" > VERSION
printf 'version-bump: %s -> %s\n' "$old" "$new"
