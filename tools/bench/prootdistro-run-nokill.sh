#!/bin/sh
# Same as prootdistro-run.sh but WITHOUT --kill-on-exit.
#
# Why: the idle soak of the proot-distro profile ends after 2-4 minutes with that flag
# (the session is torn down while the guest is still inside `sleep`), so a 30-minute
# idle measurement is impossible with it. This file exists to get that number; every
# timed benchmark still uses prootdistro-run.sh.
#
# Original header follows: proot-distro's invocation profile - same proot binary, different flags
# (--kill-on-exit --sysvipc, no extra binds beyond the three standard ones).
B=/data/user/0/com.linuxhub.myth.husk/files/linux
D=$B/bench/proot
R=$B/distributions/ubuntu-24.04/rootfs
export LD_LIBRARY_PATH=$D
export PROOT_LOADER=$D/loader
export PROOT_TMP_DIR=$R/tmp
exec "$D/proot" --link2symlink --sysvipc -0 -r "$R" \
  -b /dev -b /proc -b /sys -w /root \
  /usr/bin/env -i HOME=/root USER=root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  LANG=C.UTF-8 TERM=xterm-256color \
  /bin/sh -c "exec $*"
