#!/bin/sh
# Start a GNOME Shell 46 Wayland session inside the guest, without a systemd/logind
# session manager. Evidence 101. Three things bite here and none of them is the
# runtime:
#   * dbus-daemon --fork leaves *several* daemons on one address on this kernel;
#     afterwards every client connects and never gets a reply. Use --nofork and
#     background it yourself.
#   * XDG_RUNTIME_DIR must be mode 700, otherwise dbus refuses to set up its
#     transient service directory and dies.
#   * the session has no logind, so wireplumber 0.4 exits 70 and RTKit/colord are
#     simply absent. That is expected; the shell still comes up.
#
# usage: sh files/gnome-session.sh [runtime-dir-name]
F=/data/data/com.linuxhub.husk/files
R=$F/rootfs-full
D=${1:-xdg-session}
export XDG_RUNTIME_DIR=$R/tmp/$D
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus
export MESA_LOADER_DRIVER_OVERRIDE=zink GALLIUM_DRIVER=zink PANVK_KBASE_DVFS=none
export VK_ICD_FILENAMES=$R/usr/share/vulkan/icd.d/husk_panvk_frist370_execva.json
export VK_DRIVER_FILES=$VK_ICD_FILENAMES
export __EGL_VENDOR_LIBRARY_FILENAMES=$R/usr/share/glvnd/egl_vendor.d/50_mesa.json
export XKB_CONFIG_ROOT=$R/usr/share/X11/xkb
export XDG_CURRENT_DESKTOP=GNOME
cd "$F" || exit 1
# exact cleanup: never pkill -f mutter, that also matches the Android app process
for p in $(ps -A -o PID,ARGS 2>/dev/null | grep -E 'usr/bin/(gnome-shell|gnome-text-editor)' | grep -v grep | awk '{print $1}'); do
    kill -9 "$p" 2>/dev/null
done
for p in $(ps -A -o PID,ARGS 2>/dev/null | grep -E 'usr/bin/dbus-daemon --session --nofork' | grep -v grep | awk '{print $1}'); do
    kill -9 "$p" 2>/dev/null
done
sleep 2
nohup sh -c "exec sh gexec.sh rootfs-full usr/bin/dbus-daemon --session --nofork --address=unix:path=$XDG_RUNTIME_DIR/bus" \
    > "$R/tmp/session-dbus.log" 2>&1 &
sleep 3
nohup sh -c "exec sh gexec.sh rootfs-full usr/bin/gnome-shell --headless --no-x11" \
    > "$R/tmp/session-shell.log" 2>&1 &
sleep 15
echo "bus=$DBUS_SESSION_BUS_ADDRESS"
ls "$XDG_RUNTIME_DIR" | grep wayland
grep -E "Running GNOME Shell|Wayland display name" "$R/tmp/session-shell.log"
