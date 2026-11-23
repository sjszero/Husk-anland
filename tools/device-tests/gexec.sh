#!/bin/sh
# usage: sh files/gexec.sh <rootfs-subdir> <cmd...>
# Guest launcher for device tests. Paths are app-absolute because the loader shim
# rewrites them. `--library-path` *replaces* the loader's default search path, so
# every directory Ubuntu searches by default has to be listed:
#   lib/aarch64-linux-gnu   merged-/usr: everything from the base packages
#   mutter-14, gnome-shell  libmutter-cogl-14 and friends
#   gvfs                    libgvfscommon.so has no version suffix and lives here
F=/data/data/com.linuxhub.husk/files
R=$F/$1; shift
export NR_ROOTFS=$R
export NR_LD=${NR_LD:-$F/libld39.so}
export GLIBC_TUNABLES=${GLIBC_TUNABLES:-glibc.pthread.rseq=0}
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
cd "$R" || exit 1
exec "$NR_LD" --library-path "$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu/mutter-14:$R/usr/lib/gnome-shell:$R/usr/lib/aarch64-linux-gnu/gvfs" "$@"
