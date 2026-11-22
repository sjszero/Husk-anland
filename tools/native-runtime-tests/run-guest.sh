#!/bin/sh
# Host side: build + install the probe APK, then run a script *inside* the guest.
#
#   ./run-guest.sh 01-functional-matrix.sh            # script name inside ../native-runtime-tests
#   WAIT=30 ENV=NR_DEBUG=1 ./run-guest.sh accept-gcc-make.sh
#
# The APK launches <nativeLibraryDir>/libld.so (the embedded loader) as
#   libld.so --library-path <root>/usr/lib/aarch64-linux-gnu <root>/usr/bin/bash <script>
# from an untrusted_app process, with NR_ROOTFS=<root> and NR_LD=<nativeLibraryDir>/libld.so.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
APKDIR=$ROOT/tools/native-runtime-probe-apk
DEV=${DEV:?usage: DEV=<adb serial> ./run-guest.sh <script>   (see adb devices)}
PKG=com.linuxhub.probe
SCRIPT=${1:?usage: run-guest.sh <script-name>}
ENVV=${ENV:-}   # e.g. ENV=NR_BOOT_DEBUG2=1 for startup tracing

# 1. push the guest-side script into the guest root (app-private dir is not
#    executable, but it is readable, which is all a script needs)
adb -s "$DEV" push "$HERE/$SCRIPT" /data/local/tmp/"$SCRIPT" >/dev/null
adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/$SCRIPT files/nrfull/root/$SCRIPT"
# optional support files (e.g. EXTRA=bench.c), pushed next to the script
for f in ${EXTRA:-}; do
  adb -s "$DEV" push "$HERE/$f" /data/local/tmp/"$f" >/dev/null
  adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/$f files/nrfull/root/$f"
done

# 2. build + install the probe APK (its libld.so is the loader under test)
cd "$APKDIR"
JAVA_HOME="${JAVA_HOME:-/Applications/Android Studio.app/Contents/jbr/Contents/Home}" \
  ./gradlew :app:assembleDebug -q 2>&1 | grep -vE '^WARNING|^$' | tail -3 || true
adb -s "$DEV" install -r app/build/outputs/apk/debug/app-debug.apk >/dev/null

# 3. run it (nativeLibraryDir changes on every reinstall, so it is read at run time)
R=/data/data/$PKG/files/nrfull/root
[ -n "$ENVV" ] && ENVFLAG="--es env $ENVV"
adb -s "$DEV" shell "am force-stop $PKG; am start -W -n $PKG/.MainActivity \
  --es exe libld.so --es root nrfull/root $ENVFLAG \
  --es args '--library-path $R/usr/lib/aarch64-linux-gnu $R/usr/bin/bash /$SCRIPT'" >/dev/null
sleep "${WAIT:-20}"
adb -s "$DEV" shell "run-as $PKG cat files/probe.txt"
