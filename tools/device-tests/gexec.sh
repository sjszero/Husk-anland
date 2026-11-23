#!/bin/sh
# usage: sh files/gexec.sh <rootfs-subdir> <cmd...>
# Guest launcher for device tests. Paths are app-absolute because the loader shim
# rewrites them. mutter-14/ and gnome-shell/ are separate library dirs on Ubuntu
# 24.04 (libmutter-cogl-14 lives there), so both are on the library path.
F=/data/data/com.linuxhub.husk/files
R=$F/$1; shift
export NR_ROOTFS=$R
export NR_LD=${NR_LD:-$F/libld39.so}
export GLIBC_TUNABLES=${GLIBC_TUNABLES:-glibc.pthread.rseq=0}
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
cd "$R" || exit 1
exec "$NR_LD" --library-path "$R/usr/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu/mutter-14:$R/usr/lib/gnome-shell" "$@"
