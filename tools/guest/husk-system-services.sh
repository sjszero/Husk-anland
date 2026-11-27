#!/bin/sh
# Start the guest services that a normal Ubuntu systemd boot would start, but
# without pretending Husk has a working systemd/PAM stack.
#
# The Android app is the real uid; the path shim gives each child a guest root
# and the default fake-root identity.  These services still need the same
# NR_ROOTFS/NR_LD/LIBS handoff as any other guest process, otherwise polkitd
# cannot resolve /etc/passwd or iio-sensor-proxy cannot see /sys.
#
# usage: husk-system-services.sh <rootfs-dir> <loader> <library-path>
set -eu

R=${1:?usage: husk-system-services.sh <rootfs> <loader> <library-path>}
L=${2:?usage: husk-system-services.sh <rootfs> <loader> <library-path>}
LIBS=${3:?usage: husk-system-services.sh <rootfs> <loader> <library-path>}
R=$(cd "$R" && pwd -P)
case "$L" in /*) ;; *) L=$(cd "$(dirname "$L")" && pwd)/$(basename "$L") ;; esac
CHILD_ROOTFS=${NR_HOST_ROOTFS:-$R}

PATH_GUEST=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games
SYSBUS=unix:path=/run/dbus/system_bus_socket

running() {
    ps -A -o args= 2>/dev/null | grep -F "$1" | grep -v grep >/dev/null 2>&1
}

start_one() {
    name=$1
    program=$2
    log=$3
    shift 3
    if [ ! -x "$R$program" ]; then
        return 0
    fi
    if running "$R$program"; then
        echo "services: $name already running"
        return 0
    fi
    # env -i is deliberate: run-as carries Android-only HOME/XDG/cache paths
    # that must not leak into a guest service.  The loader sees the relative
    # `usr/bin/env` program and the path shim resolves it inside $R.
    if [ "${NR_DESKTOP_GUEST:-0}" = 1 ]; then
        # Already inside the path shim. Exec the guest program directly so the
        # shim can select the APK-native loader once; re-execing the loader's
        # host path from this guest shell is denied by the ROM.
        setsid -d sh -c 'cd "$1" || exit 1; shift; exec "$@"' sh \
            "$R" usr/bin/env -i \
            HOME=/root PATH="$PATH_GUEST" LANG=C.UTF-8 \
            NR_ROOTFS="$CHILD_ROOTFS" NR_LD="$L" NR_LIBRARY_PATH="$LIBS" \
            NR_FAKE_UID=0 NR_FAKE_GID=0 DBUS_SYSTEM_BUS_ADDRESS="$SYSBUS" \
            "$program" "$@" \
            </dev/null >"$log" 2>&1 &
    else
        setsid -d sh -c 'cd "$1" || exit 1; shift; exec "$@"' sh \
            "$R" "$L" --library-path "$LIBS" usr/bin/env -i \
            HOME=/root PATH="$PATH_GUEST" LANG=C.UTF-8 \
            NR_ROOTFS="$CHILD_ROOTFS" NR_LD="$L" NR_LIBRARY_PATH="$LIBS" \
            NR_FAKE_UID=0 NR_FAKE_GID=0 DBUS_SYSTEM_BUS_ADDRESS="$SYSBUS" \
            "$program" "$@" \
            </dev/null >"$log" 2>&1 &
    fi
    echo "services: $name started ($log)"
}

if [ ! -S "$R/run/dbus/system_bus_socket" ]; then
    echo "services: system bus socket is absent; services not started" >&2
    exit 0
fi

# polkitd must be named before the sensor proxy's Claim* calls.  It is launched
# directly because the custom bus runs as the Android app uid and cannot use
# dbus-daemon-launch-helper's setuid path to the distro service uid.
start_one polkitd /usr/lib/polkit-1/polkitd "$R/tmp/husk-polkitd.log" --no-debug

# The D-Bus service file is still present for clients that activate it, but a
# direct start makes the first monitor-sensor call reliable on clean sessions.
start_one iio-sensor-proxy /usr/libexec/iio-sensor-proxy "$R/tmp/husk-iio-sensor-proxy.log"
