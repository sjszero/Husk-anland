#!/bin/sh
# Host side: run a script *inside* the guest, in the App that is already
# installed on the device. Nothing is built, packaged or installed here - the
# whole point of Husk is that the guest is just processes in the App's own uid,
# so a test is a `run-as` away.
#
#   DEV=<adb serial> ./run-guest.sh 01-functional-matrix.sh
#   DEV=... ENV=NR_DEBUG=1 ./run-guest.sh accept-gcc-make.sh
#
# The script is pushed into the guest root and executed by the guest's own bash:
#   libld.so --library-path <root>/lib... <root>/usr/bin/bash /<script>
# with NR_ROOTFS=<root> and NR_LD=<loader>, exactly like the App launches it.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
DEV=${DEV:?usage: DEV=<adb serial> ./run-guest.sh <script>   (see adb devices)}
PKG=${PKG:-com.linuxhub.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
SCRIPT=${1:?usage: run-guest.sh <script-name>}
ENVV=${ENV:-}   # e.g. ENV=NR_SIGSYS=log for startup tracing

push() {   # push <local> <name-in-guest-root>
  adb -s "$DEV" push "$1" "/data/local/tmp/$2" >/dev/null 2>&1
  adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/$2 $ROOTFS/$2"
  adb -s "$DEV" shell "run-as $PKG chmod 755 $ROOTFS/$2"
}

# The exec helper and the script both have to exist inside the rootfs: files in
# /data/local/tmp are not visible to the guest (its / is the rootfs), and the
# App's private dir is where run-as already starts.
push "$HERE/husk-exec.sh" tmp/husk-exec.sh
push "$HERE/$SCRIPT" "$SCRIPT"
for f in ${EXTRA:-}; do push "$HERE/$f" "tmp/$(basename "$f")"; done

EXPORTS=
if [ -n "$ENVV" ]; then
  for kv in $(echo "$ENVV" | tr ',' ' '); do EXPORTS="$EXPORTS $kv"; done
fi
# shellcheck disable=SC2086
adb -s "$DEV" shell "run-as $PKG env$EXPORTS sh $ROOTFS/tmp/husk-exec.sh $ROOTFS $LD usr/bin/bash /$SCRIPT" 2>&1
