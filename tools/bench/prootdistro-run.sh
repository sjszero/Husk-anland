#!/bin/sh
# Runner: proot-distro's invocation profile - same proot binary, different flags
# (--kill-on-exit --sysvipc, no extra binds beyond the three standard ones).
B=/data/user/0/com.linuxhub.husk/files/linux
D=$B/bench/proot
R=$B/distributions/ubuntu-24.04/rootfs
export LD_LIBRARY_PATH=$D
export PROOT_LOADER=$D/loader
export PROOT_TMP_DIR=$R/tmp
exec "$D/proot" --kill-on-exit --link2symlink --sysvipc -0 -r "$R" \
  -b /dev -b /proc -b /sys -w /root \
  /usr/bin/env -i HOME=/root USER=root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  LANG=C.UTF-8 TERM=xterm-256color \
  /bin/sh -c "exec $*"
