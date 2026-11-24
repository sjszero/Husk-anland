#!/usr/bin/env python3
"""Build the eight-dimension comparison table from three interleaved bench logs.

    tools/bench/compare.py bench-husk-*.log bench-proot-*.log bench-proot-distro-*.log

Prints a markdown table: one row per measured item, grouped by the dimension it
belongs to, with the median of each runner and the "Husk relative to proot"
factor (proot / Husk, so >1 means Husk is faster). Items missing for a runner
print `n/a`; nothing is averaged across runners and no value is filled in.
"""
import statistics
import sys
from collections import defaultdict

DIMENSIONS = [
    ("1 · 启动", ["cold_start_ms", "apt_update_cold", "apt_update_hot"]),
    ("2 · 进程与 syscall 密度", ["fork_exec_100", "fork_50_true", "sh_loop_1000",
                                 "getpid_20k", "stat_20k", "open_close_20k", "make_j4"]),
    ("3 · 文件 I/O", ["find_usr", "tar_doc", "grep_include", "write_256m",
                      "read_256m", "small_files", "git_clone"]),
    ("4 · 包管理与开发工作流", ["dpkg_l", "apt_simulate", "gcc_hello",
                                "python_json", "python_numpy"]),
]
UNIT = {
    "cold_start_ms": "ms", "apt_update_cold": "ms", "apt_update_hot": "ms",
    "fork_exec_100": "ms", "fork_50_true": "ms", "sh_loop_1000": "ms",
    "getpid_20k": "ms", "stat_20k": "ms", "open_close_20k": "ms", "make_j4": "ms",
    "find_usr": "ms", "tar_doc": "ms", "grep_include": "ms", "write_256m": "ms",
    "read_256m": "ms", "small_files": "ms", "git_clone": "ms",
    "dpkg_l": "ms", "apt_simulate": "ms", "gcc_hello": "ms",
    "python_json": "ms", "python_numpy": "ms",
}
LABEL = {
    "cold_start_ms": "冷启动到 `/bin/true`（含 adb）",
    "apt_update_cold": "`apt-get update` 冷（删索引）",
    "apt_update_hot": "`apt-get update` 热",
    "fork_exec_100": "fork+exec × 100",
    "fork_50_true": "`/bin/true` × 50",
    "sh_loop_1000": "shell 循环 1000 次",
    "getpid_20k": "`getpid` × 20 000",
    "stat_20k": "`stat` × 20 000",
    "open_close_20k": "`open`+`close` × 20 000",
    "make_j4": "`make -j4`（200 目标链接）",
    "find_usr": "`find /usr`",
    "tar_doc": "源码树 tar 解包",
    "grep_include": "`grep -r` 头文件树",
    "write_256m": "顺序写 256 MB",
    "read_256m": "顺序读 256 MB（页缓存）",
    "small_files": "1 万小文件建删",
    "git_clone": "本地 `git clone` + `status`",
    "dpkg_l": "`dpkg -l`",
    "apt_simulate": "`apt-get -s install` 解析",
    "gcc_hello": "`gcc` 编译 hello",
    "python_json": "`python3` import json",
    "python_numpy": "`python3` import numpy",
}


def medians(path):
    cur, per = None, defaultdict(list)
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.rstrip("\n")
            if line.startswith("#rep"):
                cur = {}
                continue
            if line.startswith("#") or "\t" not in line:
                continue
            name, _, val = line.partition("\t")
            try:
                per[name].append(float(val))
            except ValueError:
                continue
    return {k: (statistics.median(v), min(v), max(v), len(v))
            for k, v in per.items() if v and statistics.median(v) >= 0}


def cell(m, key):
    if key not in m:
        return "n/a", None
    med, lo, hi, n = m[key]
    return f"{med:,.0f} {UNIT.get(key, '')}".strip(), med


def main(argv):
    if len(argv) != 3:
        print(__doc__)
        return 2
    h, p, d = (medians(a) for a in argv)
    print("| 维度 / 项 | Husk | proot | proot-distro | Husk 相对 proot |")
    print("|---|---:|---:|---:|---:|")
    for dim, keys in DIMENSIONS:
        print(f"| **{dim}** | | | | |")
        for k in keys:
            ch, vh = cell(h, k)
            cp, vp = cell(p, k)
            cd, _ = cell(d, k)
            ratio = f"{vp / vh:.1f}×" if (vh and vp) else "—"
            print(f"| {LABEL.get(k, k)} | {ch} | {cp} | {cd} | {ratio} |")
    print("| **8 · 兼容性** | | | | |")
    for k in sorted(set(h) | set(p) | set(d)):
        if not k.startswith("have_"):
            continue
        print(f"| `{k[5:]}` | " + " | ".join(
            "可用" if m.get(k, (0,))[0] == 1 else "无" for m in (h, p, d)) + " | — |")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
