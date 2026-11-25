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
# Only the *loader's* own program argument must be relative (usr/bin/env); the
# loader rejects absolute program paths. Everything after env is an ordinary
# guest exec, so the guest program is normalised to an absolute path below -
# which it must be anyway now that env chdirs with -C.
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
# a leading "--" from the caller is a separator, not a program name
[ "${1:-}" = "--" ] && shift
[ $# -gt 0 ] || { echo "husk-session: no command given" >&2; exit 64; }

NR_LD=${NR_LD:-$(dirname "$ROOTFS")/libld.so}
[ -x "$NR_LD" ] || { echo "husk-session: loader not executable: $NR_LD" >&2; exit 69; }

# The guest libc is not stock: it carries the shim's slot page and clone3 ->
# ENOSYS (docs/LIMITATIONS.md 4). `apt upgrade` of libc6 replaces it with the
# distro's own copy and the symptom appears much later and far away from the
# cause: empty `getent shadow`, failing `su`, and `/bin/sh: Cannot fork`. Check
# the hash the installer recorded, and put the bundled copy back if it moved.
STATE=$(dirname "$ROOTFS")/.husk-install-state
LIBC=$ROOTFS/usr/lib/aarch64-linux-gnu/libc.so.6
if [ -f "$STATE" ] && [ -f "$LIBC" ]; then
    want=$(sed -n 's/^libc_sha256=//p' "$STATE" | head -1)
    if [ -n "${want:-}" ] && [ "$want" != unknown ]; then
        have=$(sha256sum "$LIBC" | cut -d' ' -f1)
        if [ "$want" != "$have" ]; then
            bundled=$(dirname "$ROOTFS")/libc.so.6
            if [ -f "$bundled" ] && [ "$(sha256sum "$bundled" | cut -d' ' -f1)" = "$want" ]; then
                cp "$bundled" "$LIBC" && chmod 755 "$LIBC"
                echo "husk-session: libc was $(echo "$have" | cut -c1-12), not the installed $(echo "$want" | cut -c1-12); restored" >&2
            else
                echo "husk-session: guest libc is $(echo "$have" | cut -c1-12) but this install recorded" \
                     "$(echo "$want" | cut -c1-12), and no matching libc.so.6 sits next to the rootfs." >&2
                echo "              an apt upgrade of libc6 does this; rebuild the install, or re-run" >&2
                echo "              `make libc-patch` on the dev box (or re-install the APK)." \
                     "Refusing to start a broken tree." >&2
                exit 74
            fi
        fi
    fi
fi

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

# GPU: when the tree carries the kbase-backed PanVK ICD, point Vulkan at it and
# send GL through Zink, so EGL/GL calls execute on the Mali GPU instead of
# llvmpipe (evidence 156: renderer goes from "llvmpipe (LLVM …)" to
# "zink Vulkan 1.3(Mali-G77 MC9)"). HUSK_GPU=llvmpipe keeps the software path,
# which is what you want while bisecting a rendering bug or on a tree without the
# ICD. The API ceiling of that combination is GLES 2.0 for now (LIMITATIONS 7).
gpu_env=
icd=usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
if [ -r "$ROOTFS/$icd" ] && [ "${HUSK_GPU:-zink}" != llvmpipe ]; then
    gpu_env="VK_ICD_FILENAMES=/$icd VK_DRIVER_FILES=/$icd"
    gpu_env="$gpu_env MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink"
fi

# Two things every desktop session needs and that nothing else creates.
# (evidence 191: both were hit one after the other while restarting a session for real)
#
# 1. ~/Desktop. DING (the desktop-icons process) refuses to start when the
#    folder is missing -- "Error when getting information for file /root/Desktop:
#    No such file or directory" -- and with DING gone there is no wallpaper and
#    no icons, only the solid primary-color. Creating it is free and idempotent.
#    DING also refuses to let you launch anything when the folder is world
#    writable ("Desktop is writable by others - will not allow launching any
#    desktop files"), which is what `mkdir` under umask 0 produces -- so the mode
#    is set explicitly instead of inherited.
mkdir -p "$ROOTFS$home/Desktop" 2>/dev/null || true
chmod 755 "$ROOTFS$home/Desktop" 2>/dev/null || true

# 2. System bus. gnome-shell aborts during init when accountsservice cannot be
#    reached: "Couldn't connect to system bus ... Execution of main.js threw
#    exception". The tree carries the config; start it only when the socket is
#    gone, and point the child at it either way.
if [ ! -S "$ROOTFS/run/dbus/system_bus_socket" ]; then
    mkdir -p "$ROOTFS/run/dbus" 2>/dev/null || true
    rm -f "$ROOTFS/run/dbus/system_bus_socket" "$ROOTFS/run/dbus/pid" 2>/dev/null || true
    ( cd "$ROOTFS" && "$NR_LD" --library-path "$LIBS" usr/bin/env -i \
        HOME=/root PATH="$PATH_GUEST" NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" \
        /usr/bin/dbus-daemon \
        --config-file=/usr/local/share/linuxhub/linuxhub-system-bus.conf \
        --nofork --address=unix:path=/run/dbus/system_bus_socket \
        >"$ROOTFS/tmp/husk-sysbus.log" 2>&1 & ) || true
    i=0
    while [ $i -lt 30 ] && [ ! -S "$ROOTFS/run/dbus/system_bus_socket" ]; do
        sleep 0.2; i=$((i+1))
    done
fi

cd "$ROOTFS"
# env is started by the loader with a relative path; everything after it is a
# normal exec (absolute guest paths are fine there).
# the loader runs env by a relative path; from here on, guest paths are absolute
case "${1:-}" in
    /*) ;;
    *)  prog=$1; shift; set -- "/$prog" "$@" ;;
esac

# -C is a *guest* chdir: env runs inside the path layer, so "/home/hu1" is
# resolved against the rootfs. A host-side `cd` here would leave the guest.
# Desktop icons + wallpaper fallback. gnome-shell's own DING launch hands over an
# empty -P (evidence 191 §4), so DING dies on `Gtk.CssProviderError: ... /stylesheet.css`
# and the desktop loses BOTH the icons and the wallpaper (DING draws the wallpaper).
# The watcher only steps in when no "Desktop Icons" window shows up, so it is a
# no-op on a healthy session.
case "${1:-}" in
    */gnome-shell)
        [ -x "$BASE/husk-ding-fallback.sh" ] && \
            { "$BASE/husk-ding-fallback.sh" "$ROOTFS" "$NR_LD" >/dev/null 2>&1 & } || true
        # 壁纸先缩到屏幕尺寸：不缩放就不会踩到 zink/PanVK 那条"右侧三角只剩 32% 亮度"
        # 的路（evidence 195）。纯色底不动，失败静默。
        [ -x "$BASE/husk-wallpaper-fit.sh" ] && \
            { "$BASE/husk-wallpaper-fit.sh" "$ROOTFS" "$NR_LD" "$home" >/dev/null 2>&1 & } || true
        ;;
esac

exec "$NR_LD" --library-path "$LIBS" usr/bin/env -i -C "$CWD" \
    HOME="$home" USER="$USER_NAME" LOGNAME="$USER_NAME" SHELL="$shell" \
    PATH="$PATH_GUEST" LANG=C.UTF-8 TERM="${TERM:-xterm-256color}" COLORTERM=truecolor \
    NR_ROOTFS="$ROOTFS" NR_LD="$NR_LD" \
    DBUS_SYSTEM_BUS_ADDRESS="unix:path=/run/dbus/system_bus_socket" \
    NR_FAKE_UID="$uid" NR_FAKE_GID="$gid" \
    $gpu_env \
    $force_gpu \
    "$@"
