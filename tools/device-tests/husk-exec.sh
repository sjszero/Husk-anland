#!/bin/sh
# Guest launcher for the device tests. Runs inside the App's own uid, so it
# needs no APK of its own; this file is pushed next to the test script and
# executed as `run-as <pkg> sh <rootfs>/tmp/husk-exec.sh <rootfs> <loader> <prog>`.
#
# Four things the loader needs and none of them are defaults:
#   * cwd inside the guest root - the program argument is guest-relative
#     ("usr/bin/bash") and is resolved against it;
#   * --library-path, which *replaces* the loader's search path, so the guest's
#     standard directories and every directory its own binaries declare in
#     DT_RUNPATH go here (glibc resolves RUNPATH inside the loader, where nothing
#     can interpose - evidence 139);
#   * NR_ROOTFS and NR_LD both set. Without NR_ROOTFS the parent process does no
#     rewriting at all; without NR_LD a re-entering child falls back to the
#     distro loader;
#   * the app process' HOME/XDG values dropped, or dconf creates
#     $HOME/.cache/dconf *inside* the rootfs as rootfs/data/user/0/... (measured).
# usage: husk-exec.sh <rootfs> <loader> <program> [args...]
R=$1; L=$2; shift 2
# Both come in relative to the app's data directory (what run-as starts in): make
# them absolute before the cd below, or the loader path breaks with it.
case $R in /*) ;; *) R=$PWD/$R ;; esac
case $L in /*) ;; *) L=$PWD/$L ;; esac
E=
[ -f "$R/etc/ld.so.husk-path" ] && E=$(sed -n 's|^|'"$R"'|p' "$R/etc/ld.so.husk-path" | tr '\n' ':')
cd "$R" || exit 1
unset XDG_CACHE_HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME
# NR_LIBRARY_PATH is the launcher-to-shim contract for *nested* execs (the app
# exports it). Without it a child falls back to the shim's built-in list plus
# /etc/ld.so.husk-path, which is close but not what the app does - and the tests
# have to measure the app's launch, not a cousin of it.
LP="$R/lib/aarch64-linux-gnu:$R/usr/lib/aarch64-linux-gnu:$E"
# Verify the host-path translation before exec. Replacing only the leading slash
# produced `.../rootusr/lib/...`; the loader then reported a missing library that
# was visibly present in the tree. Each declared guest directory must appear as
# the exact `<root><guest path>` component in --library-path.
if [ -f "$R/etc/ld.so.husk-path" ]; then
    while IFS= read -r d; do
        case "$d" in ''|'#'*) continue ;; esac
        expected=$R$d
        case ":$LP:" in
            *":$expected:"*) ;;
            *) echo "husk-exec: malformed library path for $d (expected $expected)" >&2; exit 78 ;;
        esac
    done < "$R/etc/ld.so.husk-path"
fi
HOME=/root \
  PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  NR_ROOTFS=$R NR_LD=$L NR_LIBRARY_PATH="$LP" \
  exec "$L" --library-path "$LP" "$@"
