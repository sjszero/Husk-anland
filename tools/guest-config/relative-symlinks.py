#!/usr/bin/env python3
"""Rewrite every absolute symlink inside a guest rootfs into a relative one.

Why this is not cosmetic: the guest loader is a *host* process. When glibc's
`_dl_map_object` searches `--library-path`, it builds `<dir>/<soname>` where
`<dir>` is a host path, and the kernel resolves a symlink target against the
*host* root. An absolute target such as `/etc/alternatives/libblas.so.3-aarch64-linux-gnu`
therefore escapes the rootfs and the library "cannot be found", even though
`readlink -f` inside the guest resolves it fine.

A relative target resolves identically in both namespaces, so normalising costs
nothing and removes the whole failure class. Measured 2026-11-24:
`ctypes.CDLL('libblas.so.3')` failed with ENOENT until both hops of the
`/etc/alternatives` chain were relative, and then `import numpy` got one library
further (to liblapack) and failed the same way.

    relative-symlinks.py [--apply] [--root /]

Without --apply it only reports. Pseudo-filesystems are skipped; a target that
does not resolve inside the root is left alone and listed.
"""
import argparse
import os
import sys

SKIP_TOP = {"proc", "sys", "dev", "run", "tmp"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default="/")
    ap.add_argument("--apply", action="store_true")
    args = ap.parse_args()

    root = os.path.abspath(args.root)
    total = absolute = rewritten = dangling = 0
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        if dirpath == root:
            dirnames[:] = [d for d in dirnames if d not in SKIP_TOP]
        for name in dirnames + filenames:
            path = os.path.join(dirpath, name)
            if not os.path.islink(path):
                continue
            total += 1
            target = os.readlink(path)
            if not target.startswith("/"):
                continue
            absolute += 1
            # Resolve inside the guest root, not the host: strip the root prefix
            # before asking the host filesystem anything.
            host_target = os.path.join(root, target.lstrip("/"))
            if not os.path.exists(host_target):
                dangling += 1
                print(f"skip (dangling): {path} -> {target}")
                continue
            guest_dir = "/" + os.path.relpath(dirpath, root) if dirpath != root else "/"
            rel = os.path.relpath(os.path.normpath(target), guest_dir)
            if args.apply:
                os.unlink(path)
                os.symlink(rel, path)
            rewritten += 1
            if rewritten <= 10:
                print(f"{'rewrote' if args.apply else 'would rewrite'}: {path} -> {rel}")
    print(
        f"symlinks={total} absolute={absolute} "
        f"{'rewritten' if args.apply else 'rewritable'}={rewritten} dangling={dangling}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
