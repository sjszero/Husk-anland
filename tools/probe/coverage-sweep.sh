#!/bin/sh
# coverage-sweep.sh - 用 strace 把真实负载实际调到的路径全捞出来，
# 找出「本该被翻译、却原样交给了内核」的绝对路径。
#
# 为什么需要它：path-shim 按符号名改写 GOT/PLT，漏一个不报错，只是静默走错路。
# 点测（coverage-probe）只能验"我怀疑漏了的那几个"，这份是无偏的普查：
#   guest 里 strace 可用（ptrace 没被我们占用）→ 记录 %file 类系统调用的路径参数
#   → 凡是"在 guest rootfs 里真的存在、却没有带 rootfs 前缀"的绝对路径，就是漏网。
#
# 判据在设备上做：`[ -e <rootfs>$p ]` 走的是宿主路径（不经过 shim），所以它回答的是
# "guest 里有没有这个文件"，和 trace 里"有没有带前缀"是两件独立的事。
#
# 用法: sh tools/probe/coverage-sweep.sh <device-serial> [workload…]
set -eu
DEV=${1:?device serial}; shift || true
PKG=${PKG:-com.linuxhub.myth.husk}
APPDIR=${APPDIR:-/data/data/$PKG/files/linux}
ROOTFS=${ROOTFS:-$APPDIR/distributions/ubuntu-24.04/rootfs}
LD=${LD:-$APPDIR/husk/libld.so}
SESS=${SESS:-/data/local/tmp/husk-session.sh}
OUT=${OUT:-/tmp/husk-sweep}
mkdir -p "$OUT"

if [ $# -eq 0 ]; then
    set -- "/bin/ls -l /usr/share/doc /usr/share/locale" \
           "/usr/bin/find /etc -name '*.conf'" \
           '/usr/bin/python3 -c "import json, ssl"' \
           "/usr/bin/apt-get -s install acl"
fi

adbs() { adb -s "$DEV" shell "$@"; }
gu() { adbs "run-as $PKG env NR_LD=$LD sh $SESS $ROOTFS -- /bin/sh -c '$1'"; }

if ! gu "command -v strace >/dev/null" 2>/dev/null; then
    echo "--- coverage-sweep: SKIP  (rootfs 里没有 strace；装法见 docs/design/evidence/216)"
    exit 0
fi

i=0
for w in "$@"; do
    i=$((i+1))
    echo "== [$i] $w"
    gu "strace -f -e trace=%file -o /tmp/cov-$i.log $w >/dev/null 2>&1; echo lines=\$(wc -l < /tmp/cov-$i.log)" || true
    gu "cat /tmp/cov-$i.log" > "$OUT/cov-$i.log" 2>/dev/null || true
    echo "   拉到 $(wc -l < "$OUT/cov-$i.log" | tr -d ' ') 行"
done

python3 - "$OUT" "$ROOTFS" <<'PY' > "$OUT/candidates.txt"
import os, sys
out, rootfs = sys.argv[1], sys.argv[2]
seen = set()
for name in sorted(os.listdir(out)):
    if not name.startswith('cov-') or not name.endswith('.log'):
        continue
    for line in open(os.path.join(out, name), errors='replace'):
        # execve 的 argv 数组里也会出现路径（那是程序自己的参数，不是系统调用的路径实参），
        # 先把数组部分切掉，免得把 "/usr/bin/python3" 这种当成漏网。
        line = line.split('[', 1)[0]
        # strace 把每个路径都放在双引号里；按引号切分比正则稳（转义层数太多容易写错）
        parts = line.split('"')
        for idx in range(1, len(parts), 2):
            p = parts[idx]
            if p.startswith('/') and not p.startswith(rootfs):
                seen.add(p)
for p in sorted(seen):
    print(p)
PY
echo "== 候选路径 $(wc -l < "$OUT/candidates.txt" | tr -d ' ') 条，去设备上判「guest 里到底有没有」"
adb -s "$DEV" push "$OUT/candidates.txt" /data/local/tmp/husk-candidates.txt >/dev/null
adbs "run-as $PKG sh -c 'while IFS= read -r p; do [ -e \"$ROOTFS\$p\" ] && echo \"\$p\"; done < /data/local/tmp/husk-candidates.txt'" > "$OUT/leaks.txt" 2>/dev/null || true

if [ ! -s "$OUT/leaks.txt" ]; then
    echo "--- coverage-sweep: PASS  (没有发现未翻译的 guest 绝对路径)"
    exit 0
fi
echo "--- coverage-sweep: FAIL  $(wc -l < "$OUT/leaks.txt" | tr -d ' ') 个路径本该翻译却没翻译"
sort "$OUT/leaks.txt" | uniq -c | sort -rn | head -20 | sed 's/^/   /'
exit 1
