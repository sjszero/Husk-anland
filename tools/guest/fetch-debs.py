#!/usr/bin/env python3
"""Fetch arm64 .deb files for a guest tree from a machine with internet.

Why: the guest tree has no way to download packages when the phone is offline
(or when the app has no working DNS), but the *host* machine usually does. This
resolves a dependency closure against the Ubuntu archive indices, skips anything
the tree already has, and downloads only the missing debs so they can be pushed
into the tree and installed with `dpkg -i`.

It is deliberately a small resolver, not apt: it understands `Depends` /
`Pre-Depends` with alternatives and version constraints, ignores versions when
picking (the archive's own dependency set is self-consistent), and does not
touch the tree. Install remains an explicit step.

  ./fetch-debs.py --installed /tmp/installed.txt --out /tmp/debs xvfb xterm
  # then: adb push /tmp/debs/*.deb ...; dpkg -i in the guest

`--installed` is one package name per line or space separated
(`dpkg-query -W -f='${Package}\\n'`).
"""
import argparse
import io
import lzma
import os
import re
import sys
import urllib.request

# arm64 lives on the ports mirror; archive.ubuntu.com only mirrors it partially
# (dists/noble/Release lists binary-arm64, but the indices themselves 404 there).
MIRROR = "http://ports.ubuntu.com/ubuntu-ports"


def fetch(url):
    with urllib.request.urlopen(url, timeout=60) as r:
        return r.read()


def load_index(suite, component):
    # The archive no longer publishes Packages.gz; .xz is the one that exists.
    url = "%s/dists/%s/%s/binary-arm64/Packages.xz" % (MIRROR, suite, component)
    raw = fetch(url)
    data = lzma.decompress(raw).decode("utf-8", "replace")
    out = {}
    provides = {}
    for stanza in data.split("\n\n"):
        if not stanza.strip():
            continue
        fields = {}
        for line in stanza.splitlines():
            if line.startswith(" ") or ":" not in line:
                continue
            k, v = line.split(":", 1)
            fields[k.strip()] = v.strip()
        name = fields.get("Package")
        if not name:
            continue
        out[name] = fields
        for prov in fields.get("Provides", "").split(","):
            prov = prov.split("(")[0].strip()
            if prov:
                provides.setdefault(prov, []).append(name)
    return out, provides


def deps_of(fields):
    names = []
    for key in ("Pre-Depends", "Depends"):
        for chunk in fields.get(key, "").split(","):
            chunk = chunk.strip()
            if not chunk:
                continue
            alts = []
            for alt in chunk.split("|"):
                alt = alt.split("(")[0].strip()
                alt = alt.split(":")[0].strip()
                if alt:
                    alts.append(alt)
            if alts:
                names.append(alts)
    return names


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--suite", default="noble")
    ap.add_argument("--components", default="main,universe")
    ap.add_argument("--installed", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("packages", nargs="+")
    a = ap.parse_args()

    installed = set()
    with open(a.installed) as f:
        for tok in f.read().split():
            installed.add(tok.strip())

    index = {}
    provides = {}
    for comp in a.components.split(","):
        idx, prov = load_index(a.suite, comp)
        index.update(idx)
        for k, v in prov.items():
            provides.setdefault(k, []).extend(v)

    os.makedirs(a.out, exist_ok=True)
    todo = list(a.packages)
    seen = set()
    resolved = []
    missing = []
    while todo:
        name = todo.pop(0)
        if name in seen:
            continue
        seen.add(name)
        if name in installed:
            continue
        if name not in index:
            # virtual package: take the first provider
            if name in provides:
                todo.insert(0, provides[name][0])
                continue
            missing.append(name)
            continue
        fields = index[name]
        resolved.append(name)
        for alts in deps_of(fields):
            pick = None
            for alt in alts:
                if alt in installed or alt in seen:
                    pick = None
                    break
                if alt in index or alt in provides:
                    pick = alt
                    break
            if pick:
                todo.append(pick)

    if missing:
        print("MISSING in %s/%s: %s" % (a.suite, a.components, " ".join(sorted(set(missing)))),
              file=sys.stderr)
    total = 0
    for name in sorted(resolved):
        fn = index[name]["Filename"]
        ver = index[name]["Version"]
        dest = os.path.join(a.out, os.path.basename(fn))
        if os.path.exists(dest):
            print("have %s %s" % (name, ver))
            continue
        data = fetch("%s/%s" % (MIRROR, fn))
        with open(dest, "wb") as f:
            f.write(data)
        total += len(data)
        print("get  %s %s (%d bytes)" % (name, ver, len(data)))
    print("resolved=%d downloaded=%.1f MiB -> %s" % (len(resolved), total / 1048576.0, a.out))
    return 0 if not missing else 1


if __name__ == "__main__":
    sys.exit(main())
