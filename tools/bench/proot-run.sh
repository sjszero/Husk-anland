#!/bin/sh
# Runner: proot, with the exact flags LinuxHub 2.0 used (prootCommandLegacy).
B=/data/user/0/com.linuxhub.myth.husk/files/linux
D=$B/bench/proot
R=$B/distributions/ubuntu-24.04/rootfs
export LD_LIBRARY_PATH=$D
export PROOT_LOADER=$D/loader
export PROOT_TMP_DIR=$R/tmp
exec "$D/proot" --link2symlink -0 -r "$R" \
  -b /dev -b "$R/dev/shm:/dev/shm" -b /proc -b /sys \
  -w /root /usr/bin/env -i HOME=/root USER=root \
  PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games \
  LANG=C.UTF-8 TERM=xterm-256color COLORTERM=truecolor \
  /bin/sh -c "exec $*"
