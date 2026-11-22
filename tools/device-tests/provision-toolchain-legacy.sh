#!/bin/sh
# Provision a C toolchain (gcc 13, binutils, make, a POSIX shell) inside the
# guest rootfs that lives in the app's own private storage.
#
# Everything is resolved from ports.ubuntu.com for the same suite the rootfs was
# built from, unpacked on the host, and pushed as a tar stream through
# `run-as`. No root, no chroot, no namespaces.
#
#   ./provision-toolchain.sh              # resolve, download, unpack, push
#   NO_PUSH=1 ./provision-toolchain.sh    # stop before pushing
#
# The libc6 package is deliberately skipped: the rootfs already carries a libc
# whose optional syscalls have been rewritten (tools/loader/nr-scan-syscalls.py),
# and replacing it would undo that.
#
# Version pins live in one place so the evidence stays reproducible.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)

SUITE=${SUITE:-noble}
SUITE_UPDATES=${SUITE_UPDATES:-$SUITE-updates}
ARCH=${ARCH:-arm64}
PKGS=${PKGS:-"gcc make binutils libc6-dev dash sed grep tar gzip findutils diffutils"}
STAGE=${STAGE:-/tmp/nr-toolchain/root}
CACHE=${CACHE:-/tmp/nr-toolchain/deb}
SKIP=${SKIP:-libc6}
ADB=${ADB:-adb}
SERIAL=${SERIAL:-}
APP=${APP:-com.linuxhub.probe}
GUEST_ROOT=${GUEST_ROOT:-files/nrfull/root}

mkdir -p "$STAGE" "$CACHE"

python3 - "$SUITE" "$SUITE_UPDATES" "$ARCH" "$CACHE" "$SKIP" "$PKGS" "$STAGE" <<'PY'
import gzip, io, lzma, os, re, subprocess, sys, tarfile, urllib.request

suite, suite_upd, arch, cache, skip, pkgs, stage = sys.argv[1:8]
skip = set(x for x in skip.split(",") if x)
INDEX = "http://ports.ubuntu.com/ubuntu-ports/dists/{s}/{c}/binary-%s/Packages.gz" % arch
POOL = "http://ports.ubuntu.com/ubuntu-ports/"
COMPS = ["main", "universe"]

index = {}
for s in (suite, suite_upd):
    for c in COMPS:
        url = INDEX.format(s=s, c=c)
        try:
            raw = urllib.request.urlopen(url, timeout=120).read()
        except Exception as e:
            print("skip index", url, e)
            continue
        for block in gzip.decompress(raw).decode("utf-8", "replace").split("\n\n"):
            fields = {}
            key = None
            for line in block.split("\n"):
                if line[:1] in (" ", "\t"):
                    continue
                if ":" in line:
                    k, v = line.split(":", 1)
                    fields[k.strip()] = v.strip()
            name = fields.get("Package")
            if not name:
                continue
            # a later suite (noble-updates) wins when its version is higher
            if name not in index or fields.get("Version", "") > index[name].get("Version", ""):
                index[name] = fields

def deps(name):
    f = index.get(name)
    if not f:
        return []
    out = []
    for part in f.get("Depends", "").split(","):
        part = part.strip()
        if not part:
            continue
        alts = [a.strip() for a in part.split("|")]
        out.append([re.sub(r"\s*\(.*\)", "", a).split(":")[0] for a in alts])
    return out

want = []
queue = [p for p in pkgs.split() if p]
seen = set()
while queue:
    n = queue.pop()
    if n in seen or n in skip:
        continue
    if n not in index:
        print("MISSING", n)
        continue
    seen.add(n)
    want.append(index[n])
    for alts in deps(n):
        if any(a in seen or a in skip for a in alts):
            continue
        queue.append(alts[0])

want.sort(key=lambda f: f["Package"])
print("%d packages, %.1f MB" % (len(want), sum(int(f.get("Size", 0)) for f in want) / 1e6))

def ar_members(path):
    d = open(path, "rb").read()
    assert d[:8] == b"!<arch>\n", path + ": not an ar archive"
    p, out = 8, {}
    while p + 60 <= len(d):
        hdr = d[p:p + 60]
        name = hdr[:16].decode().strip().rstrip("/")
        size = int(hdr[48:58].decode().strip())
        out[name] = d[p + 60:p + 60 + size]
        p += 60 + size + (size & 1)
    return out

def extract(payload, name):
    if name.endswith(".zst"):
        tmp = "/tmp/nr-toolchain/data.tar"
        zst = "/tmp/nr-toolchain/data.tar.zst"
        open(zst, "wb").write(payload)
        subprocess.run(["zstd", "-d", "-f", zst, "-o", tmp], check=True)
        tf = tarfile.open(tmp)
    else:
        tf = tarfile.open(fileobj=io.BytesIO(payload))
    tf.extractall(stage)

for f in want:
    deb = os.path.join(cache, os.path.basename(f["Filename"]))
    if not os.path.exists(deb) or os.path.getsize(deb) != int(f.get("Size", 0)):
        print("fetch", f["Package"], f["Version"])
        urllib.request.urlretrieve(POOL + f["Filename"], deb)
    members = ar_members(deb)
    data = members.get("data.tar.xz") or members.get("data.tar.zst") or members.get("data.tar.gz")
    if data is None:
        print("no data.tar in", deb)
        continue
    extract(data, "data.tar.xz" if "data.tar.xz" in members else
                  "data.tar.zst" if "data.tar.zst" in members else "data.tar.gz")
print("unpacked into", stage)
PY

# Generic program names. Ubuntu's /usr/bin is usrmerge-linked, so /bin and /sbin
# are the same directory and a package that ships only "aarch64-linux-gnu-gcc-13"
# still has to answer to "gcc".
cd "$STAGE/usr/bin" 2>/dev/null || { echo "no usr/bin in $STAGE"; exit 1; }
for t in as ld ar ranlib nm objcopy objdump strip readelf addr2line size; do
    [ -e "$t" ] || ln -s "aarch64-linux-gnu-$t" "$t"
done
for pair in gcc:gcc-13 cc:gcc-13 cpp:cpp-13; do
    d=${pair%%:*}; s=${pair##*:}
    [ -e "$d" ] || ln -s "$s" "$d"
done
[ -e gcc-13 ] || ln -s aarch64-linux-gnu-gcc-13 gcc-13
[ -e cpp-13 ] || ln -s aarch64-linux-gnu-cpp-13 cpp-13
# dash ships /usr/bin/sh -> dash; that is what make(1) runs recipes with.
[ -e sh ] || { [ -e dash ] && ln -s dash sh; }

echo "staged: $(du -sh "$STAGE" | cut -f1), $(find "$STAGE/usr/bin" -maxdepth 1 | wc -l | tr -d ' ') entries in usr/bin"

[ -n "$NO_PUSH" ] && exit 0

if [ -n "$SERIAL" ]; then ADB="$ADB -s $SERIAL"; fi
echo "pushing to $APP:$GUEST_ROOT"
# The tar package ships /etc/rmt, a symlink to /usr/sbin/rmt. It is only used by
# rsh-based tape drives, and the device-side extractor refuses absolute symlinks
# pointing outside the destination, so drop it.
rm -f "$STAGE/etc/rmt"

# Packages ship hard links (gunzip -> uncompress) and the app-private storage
# refuses to create them, so break every link into a private copy first. Doing it
# here rather than with a tar switch keeps the push command identical on macOS
# and Linux (bsdtar spells that switch differently, or not at all).
find "$STAGE" -type f -links +1 -exec sh -c 'cp -p "$1" "$1.__nr" && mv "$1.__nr" "$1"' _ {} \;

( cd "$STAGE" && tar czf - . ) |
    $ADB shell "run-as $APP sh -c 'cd $GUEST_ROOT && tar xzf - && echo PUSHED'"
