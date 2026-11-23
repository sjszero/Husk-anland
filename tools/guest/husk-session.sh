#!/bin/sh
# husk-session.sh - the replacement for LinuxHub 2.0's prootCommand().
#
# 2.0 builds every session as:
#   libproot.so --link2symlink -0 -r <rootfs> -b ... -w /root \
#     /usr/bin/env -i HOME=... USER=... PATH=... TERM=... <cmd>
# and then `su - <user>` for a login shell. Every one of those switches exists
# because of something proot cannot do; Husk does those things in the path
# layer, so the invocation collapses to "start the program with the guest's
# environment". See docs/design/linuxhub-2.0-swap.md for the item-by-item list.
#
# usage: husk-session.sh <rootfs-dir> [--user NAME] [--cwd DIR] -- <guest program> [args...]
#
# The guest program must be a *relative* path (usr/bin/bash); the loader takes
# relative program paths and rejects absolute ones.
#
# Environment:
#   NR_LD      our loader (default: <parent of rootfs>/libld.so)
#   NR_FORCE_SOFTWARE_GPU   pass-through test hook (see device-adapters.md)
#
# IMPORTANT: NR_ROOTFS and NR_LD must be re-exported into the child's
# environment. `env -i` (and su/login/sg) wipe the environment, and the shim
# finds the guest root exclusively through NR_ROOTFS - without it every path
# falls through to the host and the session looks like "the shim stopped
# working" (measured 2026-11-23, evidence 129).
set -eu

BASE=$(cd "$(dirname "$0")" && pwd)
[ $# -gt 0 ] || { echo "usage: husk-session.sh <rootfs> [--user N] [--cwd D] -- prog [args]" >&2; exit 64; }
ROOTFS=$(cd "$1" && pwd); shift
USER_NAME=root
CWD=
while [ $# -gt 0 ]; do
    case "$1" in
        --user) [ $# -ge 2 ] || { echo "husk-session: --user needs a value" >&2; exit 64; }; USER_NAME=$2; shift 2 ;;
        --cwd)  [ $# -ge 2 ] || { echo "husk-session: --cwd needs a value" >&2; exit 64; }; CWD=$2; shift 2 ;;
        --)     shift; break ;;
        -*)     echo "husk-session: unknown option $1" >&2; exit 64 ;;
        *)      break ;;
    esac
done
[ $# -gt 0 ] || { echo "husk-session: no command given" >&2; exit 64; }

NR_LD=${NR_LD:-$(dirname "$ROOTFS")/libld.so}
[ -x "$NR_LD" ] || { echo "husk-session: loader not executable: $NR_LD" >&2; exit 69; }

# The session identity comes from the guest's own /etc/passwd, not from the
# Android uid the app really runs as.
creds=$(grep "^$USER_NAME:" "$ROOTFS/etc/passwd" 2>/dev/null | head -1 || true)
[ -n "$creds" ] || { echo "husk-session: no such user: $USER_NAME" >&2; exit 67; }
uid=$(echo "$creds" | cut -d: -f3)
gid=$(echo "$creds" | cut -d: -f4)
home=$(echo "$creds" | cut -d: -f6)
shell=$(echo "$creds" | cut -d: -f7)
[ -n "$uid" ] || { echo "husk-session: bad passwd entry for $USER_NAME" >&2; exit 65; }
: "${home:=/root}"
: "${shell:=/bin/bash}"

[ -n "$CWD" ] || CWD=$home

PATH_GUEST=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games
LIBS="$ROOTFS/lib/aarch64-linux-gnu:$ROOTFS/usr/lib/aarch64-linux-gnu"
[ -d "$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14" ] && LIBS="$LIBS:$ROOTFS/usr/lib/aarch64-linux-gnu/mutter-14"
[ -d "$ROOTFS/usr/lib/gnome-shell" ] && LIBS="$LIBS:$ROOTFS/usr/lib/gnome-shell"
[ -d "$ROOTFS/usr/lib/aarch64-linux-gnu/gvfs" ] && LIBS="$LIBS:$ROOTFS/usr/lib/aarch64-linux-gnu/gvfs"

# An empty-but-present variable is not the same as an absent one: the GPU plan
# treats "NR_FORCE_SOFTWARE_GPU exists" as the trigger, so it is only forwarded
# when it actually has a value.
force_gpu=
if [ -n "${NR_FORCE_SOFTWARE_GPU:-}" ]; then
    force_gpu="NR_FORCE_SOFTWARE_GPU=$NR_FORCE_SOFTWARE_GPU"
fi

cd "$ROOTFS"
# env is started by the loader with a relative path; everything after it is a
# normal exec (absolute guest paths are fine there).
exec "$NR_LD" --library-path "$LIBS" usr/bin/env -i \
    HOME="$home" USER="$USER_NAME" LOGNAME="$USER_NAME" SHELL="$shell" \
    PATH="$PATH_GUEST" LANG=C.UTF-8 TERM="${TERM:-xterm-256color}" COLORTERM=truecolor \
    NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" \
    NR_FAKE_UID="$uid" NR_FAKE_GID="$gid" \
    $force_gpu \
    "$@"
