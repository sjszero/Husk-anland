#!/usr/bin/env python3
"""Turn one or more raw bench logs into a median/min/max table.

    tools/bench/summarize.py docs/design/evidence/bench-husk-*.log ...

Raw logs are the "name<TAB>value" blocks written by guest/measure.sh, one block
per repetition ("#rep N"). NA values are counted, never averaged.
"""
import glob
import statistics
import sys
from collections import defaultdict


def blocks(path):
    cur = None
    out = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.rstrip("\n")
            if line.startswith("#"):
                if line.startswith("#rep"):
                    cur = {}
                    out.append(cur)
                continue
            if "\t" not in line or cur is None:
                continue
            name, _, val = line.partition("\t")
            try:
                cur[name] = float(val)
            except ValueError:
                pass
    return out


def summarize(path):
    reps = blocks(path)
    per = defaultdict(list)
    for r in reps:
        for k, v in r.items():
            per[k].append(v)
    rows = []
    for k, vals in sorted(per.items()):
        ok = [v for v in vals if v >= 0]
        na = len(vals) - len(ok)
        if not ok:
            rows.append((k, "NA", "", "", f"{na}/{len(vals)}"))
            continue
        rows.append((
            k,
            f"{statistics.median(ok):.1f}",
            f"{min(ok):.0f}",
            f"{max(ok):.0f}",
            f"{len(ok)}/{len(vals)}",
        ))
    return rows


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    paths = []
    for a in argv:
        paths.extend(sorted(glob.glob(a)) if any(c in a for c in "*?") else [a])
    for p in paths:
        rows = summarize(p)
        label = p.rsplit("/", 1)[-1]
        print(f"\n### {label}\n")
        print(f"{'item':24}{'median':>10}{'min':>10}{'max':>10}{'n':>8}")
        for name, med, lo, hi, n in rows:
            print(f"{name:24}{med:>10}{lo:>10}{hi:>10}{n:>8}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
