#!/bin/sh
# Verify that the app session launcher (the env -i boundary) defaults to
# zh_CN.UTF-8 when the tree carries the locale, and preserves an explicit LANG.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
DEV=${DEV:?usage: DEV=<serial> sh run-session-locale-probe.sh}
PKG=${PKG:-com.linuxhub.myth.husk}
ROOTFS=${ROOTFS:-files/rootfs-full}
LD=${LD:-files/linux/husk/libld.so}
APP=/data/user/0/$PKG
case "$ROOTFS" in /*) ROOTFS_ABS=$ROOTFS ;; *) ROOTFS_ABS=$APP/$ROOTFS ;; esac
case "$LD" in /*) LD_ABS=$LD ;; *) LD_ABS=$APP/$LD ;; esac
SESSION=$(dirname "$ROOTFS_ABS")/husk-session.sh

adb -s "$DEV" push "$HERE/../guest/husk-session.sh" /data/local/tmp/husk-session-locale-probe >/dev/null
adb -s "$DEV" shell "run-as $PKG cp /data/local/tmp/husk-session-locale-probe $SESSION"
adb -s "$DEV" shell "run-as $PKG chmod 755 $SESSION"

run_env() {
  adb -s "$DEV" shell "run-as $PKG env $1 /system/bin/sh $SESSION $ROOTFS_ABS -- /usr/bin/env" 2>&1
}

default_out=$(run_env "NR_LD=$LD_ABS")
explicit_lang_out=$(run_env "LANG=en_US.UTF-8 NR_LD=$LD_ABS")
explicit_out=$(run_env "LANG=en_US.UTF-8 LANGUAGE=en_US:en NR_LD=$LD_ABS")

printf '%s\n' "$default_out" | grep -qx 'LANG=zh_CN.UTF-8' \
  || { echo "FAIL default LANG"; printf '%s\n' "$default_out" | tail -20; exit 1; }
printf '%s\n' "$default_out" | grep -qx 'LANGUAGE=zh_CN:zh' \
  || { echo "FAIL default LANGUAGE"; printf '%s\n' "$default_out" | tail -20; exit 1; }
printf '%s\n' "$explicit_lang_out" | grep -qx 'LANG=en_US.UTF-8' \
  || { echo "FAIL explicit LANG"; printf '%s\n' "$explicit_lang_out" | tail -20; exit 1; }
if printf '%s\n' "$explicit_lang_out" | grep -qx 'LANGUAGE=zh_CN:zh'; then
  echo "FAIL explicit LANG was overridden by Chinese LANGUAGE"
  printf '%s\n' "$explicit_lang_out" | tail -20
  exit 1
fi
printf '%s\n' "$explicit_out" | grep -qx 'LANG=en_US.UTF-8' \
  || { echo "FAIL explicit LANG not preserved"; printf '%s\n' "$explicit_out" | tail -20; exit 1; }
printf '%s\n' "$explicit_out" | grep -qx 'LANGUAGE=en_US:en' \
  || { echo "FAIL explicit LANGUAGE not preserved"; printf '%s\n' "$explicit_out" | tail -20; exit 1; }

echo "PASS default=zh_CN.UTF-8 explicit=en_US.UTF-8 explicit-lang-only=not-zh"
