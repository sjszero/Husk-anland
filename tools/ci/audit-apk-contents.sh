#!/bin/sh
# Assert the APK really carries the three native pieces it needs.
#
# jniLibs/{libhuskapp.so,libhuskguard.so,libld.so} are build outputs and none of
# them is tracked, so "gradlew assembleDebug succeeded" proves nothing about
# what went into the package. Missing libld.so = an APK that cannot start a
# guest at all; missing libhuskapp.so = an app with no code (hasCode=false).
set -eu
cd "$(dirname "$0")/../.."

dir=${1:-apk}
case $dir in
    apk) set -- app/app/build/outputs/apk/debug/*.apk ;;
    *) set -- "$dir"/*.apk ;;
esac
[ -f "$1" ] || { echo "no APK found in $dir" >&2; exit 1; }
apk=$1

fail=0
for lib in libhuskapp.so libhuskguard.so libld.so; do
    if unzip -l "$apk" | grep -q "lib/arm64-v8a/$lib\$"; then
        echo "OK   lib/arm64-v8a/$lib"
    else
        echo "FAIL lib/arm64-v8a/$lib missing from $apk" >&2
        fail=1
    fi
done
exit "$fail"
