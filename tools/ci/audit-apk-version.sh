#!/bin/sh
# Assert the built APK's versionName/versionCode against VERSION.
#
# Why this exists: the declared version was only checked in three places
# (VERSION, the newest CHANGELOG heading, the release tag). Gradle derives
# versionName/versionCode from ../VERSION at configure time, so the *source* is
# consistent - but nothing verified the artifact. A stale build directory, a
# hand-edited Gradle file or a rebuild from an old tree would all ship an APK
# whose version silently disagrees with the release it is attached to.
#
# versionCode is the 100-carry scheme used by `make version-bump`:
#   major*10000 + minor*100 + patch   (0.0.36 -> 36, 0.1.0 -> 100)
set -eu
cd "$(dirname "$0")/../.."

apk=${1:-}
if [ -z "$apk" ]; then
    set -- app/app/build/outputs/apk/debug/*.apk
    [ -f "$1" ] || { echo "no APK found under app/app/build/outputs/apk/debug/" >&2; exit 1; }
    apk=$1
fi
[ -f "$apk" ] || { echo "APK not found: $apk" >&2; exit 1; }

aapt2=${AAPT2:-}
if [ -z "$aapt2" ]; then
    for cand in "${ANDROID_HOME:-$HOME/Library/Android/sdk}"/build-tools/*/aapt2 \
                "${ANDROID_SDK_ROOT:-/usr/lib/android-sdk}"/build-tools/*/aapt2; do
        [ -x "$cand" ] && aapt2=$cand
    done
fi
[ -n "$aapt2" ] || { echo "aapt2 not found; set AAPT2=<path>" >&2; exit 1; }

want_name=$(cat VERSION)
want_code=$(printf '%s' "$want_name" | awk -F. '{code = 0; for (i = 1; i <= NF; i++) code = code * 100 + $i; print code}')

badging=$("$aapt2" dump badging "$apk" 2>/dev/null | head -1)
[ -n "$badging" ] || { echo "aapt2 could not read $apk" >&2; exit 1; }

got_name=$(printf '%s\n' "$badging" | tr ' ' '\n' | sed -n "s/^versionName='\(.*\)'$/\1/p")
got_code=$(printf '%s\n' "$badging" | tr ' ' '\n' | sed -n "s/^versionCode='\(.*\)'$/\1/p")

fail=0
if [ "$got_name" != "$want_name" ]; then
    echo "FAIL versionName: APK has '$got_name', VERSION says '$want_name'" >&2
    fail=1
fi
if [ "$got_code" != "$want_code" ]; then
    echo "FAIL versionCode: APK has '$got_code', 100-carry of VERSION is '$want_code'" >&2
    fail=1
fi
[ "$fail" -eq 0 ] && echo "APK version contract OK: $got_name ($got_code)"
exit "$fail"
