#!/system/bin/sh
# make-release.sh <rootfs-dir> <out.tar> — build a distributable rootfs tarball.
#
# Run ON THE DEVICE, as the app uid, against a tree that has already been
# verified (the libc patch, relative symlinks and husk-ldconfig are in it).
# The point is to hand out the tree you tested instead of a tree you rebuilt
# from packages and hope is the same.
#
# What is left out, and why:
#   tmp proc sys dev run   mount points and sockets, recreated by the installer
#   root                   whatever the person who verified the tree left there
#   var/cache/apt/archives var/lib/apt/lists   hundreds of MB of cache
#   var/log                session logs from the verifying device
#   var/lib/husk/owners    an inode->uid table: inode numbers are per-filesystem,
#                          so on another device every entry would be wrong
#   etc/ld.so.husk-path    absolute paths from the verifying device
#   etc/{hostname,resolv.conf,machine-id}  device identity
#
#   adb shell "run-as <pkg> sh <dir>/make-release.sh <rootfs> <out.tar>"
set -u
SRC=${1:?usage: make-release.sh <rootfs-dir> <out.tar>}
OUT=${2:?usage: make-release.sh <rootfs-dir> <out.tar>}
[ -d "$SRC/usr" ] || { echo "make-release: $SRC does not look like a rootfs" >&2; exit 66; }
case "$OUT" in /*) ;; *) OUT=$(pwd)/$OUT ;; esac

EX=$(dirname "$OUT")/.release-exclude
cat > "$EX" <<'LIST'
tmp
proc
sys
dev
run
root
var/cache/apt/archives
var/lib/apt/lists
var/log
var/lib/husk/owners
etc/ld.so.husk-path
etc/hostname
etc/resolv.conf
etc/machine-id
LIST

# A host path created *inside* the guest (a `mkdir -p` with the app dir spelled
# out, which the shim translates back under the rootfs) leaves a mirror of this
# app in the tree and then ships to users. None of these are Ubuntu paths, and
# gate A fails on `/data/user`, so refuse here instead of handing it out.
for junk in data storage sdcard; do
    if [ -e "$SRC/$junk" ]; then
        echo "make-release: $SRC/$junk exists - a host path was created inside the guest" >&2
        echo "              (measured 2026-11-24: a /data/user/0/<pkg>/... mirror)." >&2
        echo "              remove it and re-run; it must not ship." >&2
        exit 67
    fi
done

echo "== release tarball =="
echo "   from : $SRC"
echo "   to   : $OUT"
( cd "$SRC" && tar cf "$OUT" -X "$EX" . ) || { echo "make-release: tar failed" >&2; exit 70; }
rm -f "$EX"
echo "== result =="
ls -l "$OUT"
sha256sum "$OUT" | sed 's/^/   sha256 /'
echo "   entries: $(tar tf "$OUT" | wc -l)"
