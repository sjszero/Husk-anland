#!/bin/sh
# install-husk.sh - put a Husk rootfs + loader into a plain directory.
#
# This is the "install it like proot-distro does" path: one tarball, one script,
# no adb push per file, no root. What it needs is a writable directory and a
# loader the kernel lets us execute.
#
# The supported target is this app's own private directory (`run-as ... files/...`).
# Termux is NOT a supported environment and is not tested: the guest tree expects
# binaries and libraries that a Termux $PREFIX does not provide, so a run from
# there fails on missing dependencies rather than on anything this script can fix.
#
# usage:
#   install-husk.sh --rootfs <rootfs.tar[.gz]> [--dest DIR] [--loader PATH] [--user NAME]
#
#   --rootfs  Ubuntu base tarball (uncompressed or .gz)
#   --dest    where the rootfs goes (default: ./root next to this script)
#   --loader  our loader (default: $NR_LD, else ./libld.so next to this script)
#   --user    guest user for the generated launcher (default: root)
#
# Why two extraction passes: Ubuntu's base tarball contains hard links
# (usr/bin/uncompress -> gunzip, usr/bin/perl -> perl5.38.2, ...). Android
# refuses link(2) inside app data (SELinux), so a plain `tar xf` leaves those
# entries out. Pass 1 lays everything down and tolerates the link errors; pass 2
# re-extracts *through* the shim, whose link() falls back to a copy, so the
# missing names materialise as real files. That fallback is the same code dpkg
# relies on, so this is not a special case for installation.
set -eu

ROOTFS_TAR=
DEST=
LOADER=${NR_LD:-}
USER_NAME=root
usage() { sed -n '2,25p' "$0"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --rootfs) [ $# -ge 2 ] || { echo "--rootfs needs a value" >&2; exit 64; }; ROOTFS_TAR=$2; shift 2 ;;
        --dest)   [ $# -ge 2 ] || { echo "--dest needs a value" >&2; exit 64; };   DEST=$2; shift 2 ;;
        --loader) [ $# -ge 2 ] || { echo "--loader needs a value" >&2; exit 64; }; LOADER=$2; shift 2 ;;
        --user)   [ $# -ge 2 ] || { echo "--user needs a value" >&2; exit 64; };   USER_NAME=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "install-husk: unknown argument $1" >&2; exit 64 ;;
    esac
done

HERE=$(cd "$(dirname "$0")" && pwd)
[ -n "$ROOTFS_TAR" ] || { echo "install-husk: --rootfs is required" >&2; usage >&2; exit 64; }
# --rootfs may be a local tarball or a URL; in a terminal (Termux) a URL is the
# normal case, so that path is not a special case here.
is_url=no
case "$ROOTFS_TAR" in
    http://*|https://*) is_url=yes ;;
    *) [ -f "$ROOTFS_TAR" ] || { echo "install-husk: no such rootfs tarball: $ROOTFS_TAR" >&2; exit 66; } ;;
esac
: "${DEST:=$HERE/root}"
: "${LOADER:=$HERE/libld.so}"

# Everything below works on absolute paths: the launcher we generate has to work
# from any cwd, and half of the failure modes on Android are relative-path ones.
mkdir -p "$DEST"
DEST=$(cd "$DEST" && pwd)
OUT=$(dirname "$DEST")

# Version: one source of truth, the VERSION file shipped next to this script
# (repo root when run from a checkout). `./husk --version` prints exactly this
# string, and `make version-check` keeps it equal to the newest CHANGELOG entry.
HUSK_VERSION=${HUSK_VERSION:-}
if [ -z "$HUSK_VERSION" ]; then
    for f in "$HERE/VERSION" "$(pwd)/VERSION"; do
        if [ -f "$f" ]; then HUSK_VERSION=$(cat "$f"); break; fi
    done
fi
: "${HUSK_VERSION:=unknown}"
[ -x "$LOADER" ] || { echo "install-husk: loader not executable: $LOADER" >&2; exit 69; }
LOADER=$(cd "$(dirname "$LOADER")" && pwd)/$(basename "$LOADER")
# a URL is not a path yet; it is turned into one by the download step below
[ "$is_url" = yes ] || ROOTFS_TAR=$(cd "$(dirname "$ROOTFS_TAR")" && pwd)/$(basename "$ROOTFS_TAR")

# mksh (Android's /bin/sh) and dash both need a writable TMPDIR for `$(...)`
# capture and here-documents. Under `run-as` nothing in /tmp or /data/local is
# writable, so pick one now instead of failing half-way through the install.
for t in ${TMPDIR:-} "$DEST/tmp" "$OUT/tmp"; do
    [ -n "$t" ] || continue
    if mkdir -p "$t" 2>/dev/null && [ -w "$t" ]; then
        TMPDIR=$(cd "$t" && pwd); export TMPDIR; break
    fi
done
[ -n "${TMPDIR:-}" ] || { echo "install-husk: no writable TMPDIR" >&2; exit 73; }

echo "== husk install =="
echo "   rootfs : $ROOTFS_TAR"
echo "   dest   : $DEST"
echo "   loader : $LOADER"

if [ "$is_url" = yes ]; then
    name=$(basename "${ROOTFS_TAR%%\?*}")
    echo "== download =="
    echo "   from : $ROOTFS_TAR"
    if command -v curl >/dev/null 2>&1; then
        curl -fL --retry 2 -o "$OUT/$name.part" "$ROOTFS_TAR" || { echo "install-husk: download failed" >&2; exit 67; }
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$OUT/$name.part" "$ROOTFS_TAR" || { echo "install-husk: download failed" >&2; exit 67; }
    else
        echo "install-husk: need curl or wget to fetch a URL" >&2; exit 67
    fi
    mv "$OUT/$name.part" "$OUT/$name"
    ROOTFS_TAR=$OUT/$name
    echo "   saved: $ROOTFS_TAR"
fi

# 0. does the kernel let us execute the loader from this location? A noexec
#    mount is the single most confusing failure we have (it looks like the
#    loader is broken), so say it out loud now. Running the loader with no
#    program name prints "missing program name" and exits 1 - that output is
#    exactly what proves exec worked.
probe=$("$LOADER" 2>&1 || true)
case "$probe" in
    *"missing program name"*) ;;
    *)
        echo "install-husk: cannot execute the loader from here:" >&2
        echo "$probe" | head -3 | sed 's/^/    /' >&2
        echo "              a noexec mount looks like this; put the loader where" >&2
        echo "              exec is allowed (an app private dir under run-as allows it," >&2
        echo "              /data/local/tmp and most noexec mounts do not)." >&2
        exit 69 ;;
esac

# 1. first pass: host tar. Hard links fail on Android; that is expected.
#    Pick the decompressor by magic: toybox tar handles .tar and .tar.gz, but not
#    xz/zstd, and images in the wild come as all four. .gz goes straight through
#    tar (see the gzclose note below); the others are staged to a plain tar.
echo "== pass 1: extract (hard-link errors are expected) =="
magic=$(od -An -tx1 -N6 "$ROOTFS_TAR" | tr -d ' \n')
dec=
rc=0
case "$magic" in
    1f8b*)
        # gzip: let tar decompress. Staging the stream through `gzip -dc > file`
        # is wrong on this device: toybox gzip writes the complete file and then
        # exits 1 with "gzclose: Inappropriate ioctl for device" (measured
        # 2026-11-24), so an honest exit-code check cannot tell that apart from a
        # truncated archive.
        tar xzf "$ROOTFS_TAR" -C "$DEST" > "$OUT/.husk-pass1.log" 2>&1
        rc=$? ;;
    fd377a585a00*)  dec="xz -dc" ;;
    28b52ffd*)      dec="zstd -dc" ;;
    *)
        # Plain tar (and anything else tar can sniff on its own).
        tar xf "$ROOTFS_TAR" -C "$DEST" > "$OUT/.husk-pass1.log" 2>&1
        rc=$? ;;
esac
if [ -n "$dec" ]; then
    command -v "${dec%% *}" >/dev/null 2>&1 || { echo "install-husk: $ROOTFS_TAR is compressed but ${dec%% *} is not available" >&2; exit 67; }
    echo "   ($dec -> $OUT/.husk-pass1.tar)"
    $dec "$ROOTFS_TAR" > "$OUT/.husk-pass1.tar" || { echo "install-husk: decompression failed" >&2; exit 67; }
    case "$(od -An -tx1 -N2 "$OUT/.husk-pass1.tar" | tr -d ' \n')" in
        1f8b|fd37|28b5) echo "install-husk: $ROOTFS_TAR is compressed twice (a .gz of a .gz); pass the inner tarball" >&2
                        rm -f "$OUT/.husk-pass1.tar"; exit 67 ;;
    esac
    tar xf "$OUT/.husk-pass1.tar" -C "$DEST" > "$OUT/.husk-pass1.log" 2>&1
    rc=$?
    rm -f "$OUT/.husk-pass1.tar"
fi
sed 's/^/   /' "$OUT/.husk-pass1.log"
# Hard links inside app data fail with EACCES/EPERM because SELinux refuses
# link(2); pass 2 materialises those names through the shim. That is the *only*
# tolerated failure. A pipe used to hide tar's exit status here, so a truncated
# or corrupt archive could "succeed" on an os-release check alone.
if [ "$rc" -ne 0 ]; then
    if grep -vi 'link' "$OUT/.husk-pass1.log" | grep -qiE 'error|cannot|denied|corrupt|unexpected'; then
        echo "install-husk: pass 1 failed (tar rc=$rc); this archive is not usable" >&2
        exit 68
    fi
    echo "   (pass 1 reported link errors only; pass 2 rewrites those names)"
fi
rm -f "$OUT/.husk-pass1.log"
[ -d "$DEST/bin" ] || { echo "install-husk: pass 1 produced no /bin - bad tarball?" >&2; exit 66; }

# 2. second pass: the guest's own tar through the loader, so link() -> copy.
echo "== pass 2: re-extract through the shim (fixes hard links) =="
mkdir -p "$DEST/tmp"
cp "$ROOTFS_TAR" "$DEST/tmp/.husk-rootfs.tar"
LIBS="$DEST/lib/aarch64-linux-gnu:$DEST/usr/lib/aarch64-linux-gnu"
[ -d "$DEST/usr/lib/aarch64-linux-gnu" ] || LIBS="$DEST/usr/lib"
( cd "$DEST" && NR_ROOTFS=$DEST NR_LD=$LOADER "$LOADER" --library-path "$LIBS" usr/bin/env -i \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    NR_ROOTFS=$DEST NR_LD=$LOADER /bin/sh -c \
    'tar -xf /tmp/.husk-rootfs.tar -C / && rm -f /tmp/.husk-rootfs.tar' 2>&1 | sed 's/^/   /' )

# 2b. a release tarball leaves the mount points out on purpose
#     (tools/guest/make-release.sh): tmp/proc/sys/dev/run/root are recreated
#     here, so an install never inherits sockets or a stale /root.
mkdir -p "$DEST/dev" "$DEST/dev/shm" "$DEST/proc" "$DEST/sys" "$DEST/run" "$DEST/root" "$DEST/tmp"
chmod 1777 "$DEST/tmp"

# 2b-2. resolv.conf. A release tarball leaves it out (it is device identity), and
#       without it `apt-get update` dies on "Temporary failure resolving" while
#       the loader itself looks perfectly healthy. The app path gets this file
#       from the app; a standalone install has to write one.
if [ ! -s "$DEST/etc/resolv.conf" ]; then
    mkdir -p "$DEST/etc"
    {
        for ns in $(getprop net.dns1 2>/dev/null) $(getprop net.dns2 2>/dev/null); do
            case "$ns" in *[0-9]*) echo "nameserver $ns" ;; esac
        done
        echo "nameserver 1.1.1.1"
        echo "nameserver 8.8.8.8"
    } > "$DEST/etc/resolv.conf"
    echo "   wrote etc/resolv.conf ($(grep -c nameserver "$DEST/etc/resolv.conf") nameservers)"
fi

# 2b-2b. GPU 运行时默认值：写进 /etc/profile.d，让"进到 guest 里再开程序"也拿到
#        ICD 与 GL 驱动（husk-session.sh 覆盖的是它自己拉起的那一条会话链）。
if [ -f "$HERE/guest-config/husk-gpu-defaults.sh" ]; then
    mkdir -p "$DEST/etc/profile.d"
    cp "$HERE/guest-config/husk-gpu-defaults.sh" "$DEST/etc/profile.d/husk-gpu.sh"
    chmod 644 "$DEST/etc/profile.d/husk-gpu.sh"
    echo "   wrote etc/profile.d/husk-gpu.sh (GPU defaults, escape: HUSK_GPU=llvmpipe)"
fi

# 2b-3. log directories apt and dpkg expect. The release tarball drops var/log
#       (session logs of the verifying device); without these, every install
#       prints `E: Directory '/var/log/apt/' missing` and apt still exits with an
#       error code even though the package went in.
mkdir -p "$DEST/var/log/apt" "$DEST/var/log/apt/periodic"
: > "$DEST/var/log/dpkg.log"

# 2c. absolute symlinks. The guest loader is a host process, so the kernel
#     resolves a link target against the *host* root: /etc/alternatives/...
#     escapes the tree and dlopen() reports ENOENT for a library that is right
#     there (evidence 155). Rewrite them relative, then verify none is left.
echo "== symlinks: absolute -> relative =="
if [ -f "$HERE/husk-symlinks" ]; then
    cp "$HERE/husk-symlinks" "$DEST/usr/bin/husk-symlinks"
    chmod 755 "$DEST/usr/bin/husk-symlinks"
fi
guest_run() {
    ( cd "$DEST" && NR_ROOTFS=$DEST NR_LD=$LOADER "$LOADER" --library-path "$LIBS" usr/bin/env -i \
        PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        NR_ROOTFS=$DEST NR_LD=$LOADER "$@" )
}
if [ -x "$DEST/usr/bin/husk-symlinks" ]; then
    guest_run usr/bin/husk-symlinks --root / --apply 2>&1 | sed 's/^/   /'
    guest_run usr/bin/husk-symlinks --root / --verify --quiet 2>&1 | sed 's/^/   /' || {
        echo "install-husk: absolute symlinks still escape the rootfs; those names will not dlopen" >&2
        exit 71; }
else
    echo "   (this bundle has no husk-symlinks; absolute links, if any, stay)" >&2
fi

# 2d. the guest libc is not stock (slot page + clone3 -> ENOSYS, LIMITATIONS 4).
#     If the tree carries a different libc than the one this bundle shipped, it
#     is either a distro upgrade or a stale image; both die later with a
#     misleading `Cannot fork`. Put the bundled one back and say so.
echo "== libc =="
shipped_libc="$HERE/libc.so.6"
tree_libc="$DEST/usr/lib/aarch64-linux-gnu/libc.so.6"
[ -f "$tree_libc" ] || tree_libc="$DEST/lib/aarch64-linux-gnu/libc.so.6"
libc_sha=
if [ -f "$shipped_libc" ] && [ -f "$tree_libc" ]; then
    want=$(sha256sum "$shipped_libc" | cut -d' ' -f1)
    have=$(sha256sum "$tree_libc" | cut -d' ' -f1)
    if [ "$want" != "$have" ]; then
        echo "   tree libc $(echo "$have" | cut -c1-12) != bundle $(echo "$want" | cut -c1-12): restoring"
        cp "$shipped_libc" "$DEST/usr/lib/aarch64-linux-gnu/libc.so.6"
        [ "$tree_libc" = "$DEST/lib/aarch64-linux-gnu/libc.so.6" ] && \
            cp "$shipped_libc" "$DEST/lib/aarch64-linux-gnu/libc.so.6"
        chmod 755 "$DEST/usr/lib/aarch64-linux-gnu/libc.so.6"
        have=$(sha256sum "$DEST/usr/lib/aarch64-linux-gnu/libc.so.6" | cut -d' ' -f1)
        [ "$want" = "$have" ] || { echo "install-husk: libc restore failed" >&2; exit 72; }
    fi
    libc_sha=$want
    echo "   libc $(echo "$libc_sha" | cut -c1-12) matches the bundle"
else
    echo "   (no bundled libc.so.6 next to the installer; keeping the tree's own)" >&2
fi
{
    echo "version=$HUSK_VERSION"
    echo "libc_sha256=${libc_sha:-unknown}"
    echo "loader_sha256=$(sha256sum "$LOADER" | cut -d' ' -f1)"
    echo "installed=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$OUT/.husk-install-state"

# 3. session script + launcher, both in the parent of the rootfs. Absolute paths
#    are baked in: $0 inside the launcher is the launcher, not the rootfs.
if [ "$HERE/husk-session.sh" != "$OUT/husk-session.sh" ]; then
    cp "$HERE/husk-session.sh" "$OUT/husk-session.sh"
fi
chmod 755 "$OUT/husk-session.sh"
cat > "$OUT/husk" <<LAUNCHER
#!/bin/sh
# generated by install-husk.sh on $(date +%Y-%m-%d)
# rootfs: $DEST
# loader: $LOADER
#
#   ./husk                     - interactive shell
#   ./husk run ubuntu          - the same thing; one rootfs per install, so the
#                                name is accepted and documented, not looked up
#   ./husk run ubuntu -c 'id'  - run one command inside the guest
#   ./husk --version           - print the release this tree came from
HUSK_VERSION=$HUSK_VERSION
case "\${1:-}" in
    -V|--version) echo "husk \$HUSK_VERSION"; exit 0 ;;
    run) shift; [ \$# -gt 0 ] && shift ;;   # drop "run" and the distro name
esac
[ \$# -gt 0 ] || set -- usr/bin/bash -l
# "-c 'cmd'" (and any other leading option) is meant for the shell, not for
# husk-session.sh: that would try to exec a program called "-c".
case "\${1:-}" in -*) set -- usr/bin/bash "\$@" ;; esac
# husk-session.sh finds the loader through NR_LD; the rootfs is passed explicitly.
NR_LD="$LOADER" exec "$OUT/husk-session.sh" "$DEST" --user $USER_NAME -- "\$@"
LAUNCHER
chmod 755 "$OUT/husk"

# 4. verification that does not lie: run the guest's own tools.
echo "== verify =="
out=$( cd "$DEST" && NR_ROOTFS=$DEST NR_LD=$LOADER "$LOADER" --library-path "$LIBS" usr/bin/env -i \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    NR_ROOTFS=$DEST NR_LD=$LOADER /bin/sh -c \
    'head -1 /etc/os-release; echo "bins=$(ls /usr/bin | wc -l)"; ls -l /usr/bin/uncompress /usr/bin/perl5.38.2 2>/dev/null | sed "s/  */ /g"' 2>&1 || true )
echo "$out" | sed 's/^/   /'
case "$out" in
    *"PRETTY_NAME=\"Ubuntu"*) ;;
    *) echo "install-husk: verification failed - the guest did not answer from its own rootfs" >&2; exit 70 ;;
esac
# The hard links pass 2 exists for must now be real, readable files. Test them
# positively: a previous version grepped the combined transcript and matched the
# "cannot access" error text, i.e. it passed exactly when the file was missing.
hl=$( cd "$DEST" && NR_ROOTFS=$DEST NR_LD=$LOADER "$LOADER" --library-path "$LIBS" usr/bin/env -i \
    PATH=/usr/bin:/bin NR_ROOTFS=$DEST NR_LD=$LOADER /bin/sh -c \
    'test -s /usr/bin/perl5.38.2 && test -s /usr/bin/uncompress && echo yes || echo no' 2>&1 | tail -1 )
case "$hl" in
    yes) echo "== ok: hard links materialised (perl5.38.2 + uncompress are real files) ==" ;;
    *)   echo "install-husk: warning - hard links missing; pass 2 did not go through the shim" >&2 ;;
esac

# 4b. the check that would have caught both symlink bugs: a soname that only
#     resolves when the /etc/alternatives chain is relative.
if [ -x "$DEST/usr/bin/python3" ]; then
    son=$( guest_run /usr/bin/python3 -c \
        "import ctypes; [ctypes.CDLL(n) for n in ('libblas.so.3','liblapack.so.3')]; print('sonames ok')" \
        2>&1 | tail -2 )
    echo "$son" | sed 's/^/   /'
    case "$son" in
        *"sonames ok"*) ;;
        *) echo "install-husk: soname resolution failed (libblas/liblapack) - do not ship this tree" >&2
           exit 73 ;;
    esac
fi

cat <<DONE

== done ==
  $OUT/husk -- usr/bin/bash -l -c 'id; head -1 /etc/os-release'
DONE
