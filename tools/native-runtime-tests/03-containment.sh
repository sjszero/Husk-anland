#!/bin/bash
# Containment: what a guest path may and may not reach. Driven by run-guest.sh.
#
# Design, so the expectations below are the intended ones:
#   * the rootfs is a *root fd*, not a mount: absolute link targets restart at
#     the root, and ".." clamps there (chroot semantics without chroot);
#   * /proc, /dev and /sys are the *host's*, passed through on purpose - a guest
#     needs its own /proc/self and the device nodes the kernel actually has, and
#     an untrusted_app can only see its own process there anyway.
f=0
must_fail(){ local l=$1; shift; if "$@" >/dev/null 2>&1; then echo "ESCAPE $l"; f=$((f+1)); else echo "blocked $l"; fi; }
must_ok(){   local l=$1; shift; if "$@" >/dev/null 2>&1; then echo "ok $l";      else echo "BROKEN $l"; f=$((f+1)); fi; }

rm -rf /tmp/c; mkdir -p /tmp/c
ln -s /system/build.prop /tmp/c/abs
ln -s ../../../../../../system/build.prop /tmp/c/dots
ln -s /data/local/tmp /tmp/c/hostdir

must_fail read-abs-symlink     cat /tmp/c/abs
must_fail read-dotdot-symlink  cat /tmp/c/dots
must_fail list-hostdir-symlink ls -d /tmp/c/hostdir/..
must_fail abs-up               cat /../../../../system/build.prop
must_fail abs-up2              cat /../system/build.prop
must_fail open-escape          bash -c 'exec 9</../../../system/build.prop'
must_ok   clamp-cd-root        bash -c 'cd /..; test "$(pwd -P)" = /'
must_ok   clamp-mkdir          bash -c 'mkdir -p /../esc && test -d /esc'
must_ok   rootfs-file          test -e /etc/os-release
# host mount points pass through by design
must_ok   host-proc            bash -c 'head -1 /proc/self/status | grep -q Name'
must_ok   host-dev-null        bash -c ': > /dev/null'
echo "#escapes=$f"
[ "$f" -eq 0 ]
